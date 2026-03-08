# Plan: migrate transcription from Mistral/Voxtral to Groq Whisper Large V3 Turbo

## Goal
Replace the current Mistral/Voxtral transcription integration with Groq's `whisper-large-v3-turbo`, using `GROQ_API_KEY` and renaming the optional bias env var from `VOXTRAL_CONTEXT_BIAS` to `CONTEXT_BIAS`.

## Decision log
- User confirmed this should be a **clean breaking change**: remove the Mistral/Voxtral path instead of keeping fallback compatibility.
- Groq **does support prompt-based context/spelling guidance** for Whisper transcription via the `prompt` field, so the existing context-bias feature should be **kept**, but reworked to target Groq's `prompt` instead of Mistral's `context_bias`.

## Verified external API findings
References checked:
- Groq Speech-to-Text docs: https://console.groq.com/docs/speech-to-text
- Groq API reference (audio transcription): https://console.groq.com/docs/api-reference#audio-transcription

Confirmed request shape for `POST https://api.groq.com/openai/v1/audio/transcriptions`:
- multipart/form-data
- required fields:
  - `file` or `url`
  - `model`
- useful optional fields:
  - `language`
  - `prompt`
  - `response_format` (`json`, `text`, `verbose_json`)
  - `temperature`
  - `timestamp_granularities[]` when using `verbose_json`

Confirmed prompt/context support:
- `prompt` is supported for transcription.
- Groq docs describe it as: "guide the model's style or specify how to spell unfamiliar words" / "continue a previous audio segment".
- Prompt guidance is limited to **224 tokens**.
- This is the closest equivalent to the old context-bias capability, so `CONTEXT_BIAS` should map to `prompt`.

Important compatibility note from docs:
- Supported upload types include `ogg`, but Groq docs do **not** list `opus` as a standalone file type.
- This app already produces **Ogg Opus** data (`ogg::PacketWriter` in `run_opus_worker()`), but currently uploads it as `audio.opus` with MIME `audio/opus`.
- Plan should therefore normalize the upload metadata to Ogg, e.g. file name `audio.ogg` and MIME `audio/ogg`.

## Repo facts gathered
Relevant current code and docs:
- `src/main.rs:473` — `transcribe_audio_opus()` currently calls Mistral
- `src/main.rs:477` — reads `VOXTRAL_API_KEY`
- `src/main.rs:480` — reads `VOXTRAL_CONTEXT_BIAS`
- `src/main.rs:484-506` — builds Mistral multipart form and sends to `https://api.mistral.ai/v1/audio/transcriptions`
- `src/main.rs:489-491` — uploads file as `audio.opus` / `audio/opus`
- `CLAUDE.md` — still documents Mistral/Voxtral env vars and API
- `.env.sample` already appears to use `GROQ_API_KEY` and `CONTEXT_BIAS`, so docs/code need to be brought into alignment

---

## Implementation plan

### Phase 1 — Replace the API integration in `src/main.rs`

#### 1. Update env var handling
Edit point:
- `src/main.rs:473-481` inside `transcribe_audio_opus()`

Changes:
- Replace:
  - `VOXTRAL_API_KEY` -> `GROQ_API_KEY`
  - `VOXTRAL_CONTEXT_BIAS` -> `CONTEXT_BIAS`
- Update the panic/expect message to: `GROQ_API_KEY environment variable must be set`

Planned patch shape:
- Keep `dotenvy::dotenv().ok();`
- Load:
  - `let api_key = env::var("GROQ_API_KEY")...`
  - `let context_bias = env::var("CONTEXT_BIAS").ok();`

#### 2. Point the request to Groq and change payload fields
Edit point:
- `src/main.rs:484-506` inside `transcribe_audio_opus()`

Changes:
- Change endpoint from:
  - `https://api.mistral.ai/v1/audio/transcriptions`
  to:
  - `https://api.groq.com/openai/v1/audio/transcriptions`
- Change multipart fields:
  - `model = "voxtral-mini-latest"` -> `model = "whisper-large-v3-turbo"`
  - keep `language = "en"` for now, since the app already assumed English and Groq recommends specifying language for speed/accuracy
  - add `response_format = "json"` explicitly for stability
  - optionally add `temperature = "0"` explicitly to match Groq recommendation for deterministic transcription
- Replace unsupported Mistral field:
  - remove `.text("context_bias", bias)`
  - instead add `.text("prompt", <constructed prompt>)` if `CONTEXT_BIAS` is present and non-empty

#### 3. Fix uploaded file metadata to match actual container
Edit point:
- `src/main.rs:489-491`

Changes:
- Change uploaded file name from `audio.opus` to `audio.ogg`
- Change MIME from `audio/opus` to `audio/ogg`

Reason:
- The app sends Ogg-wrapped Opus bytes, not raw Opus.
- Groq docs list `ogg` as supported input format.

#### 4. Add a small helper for converting `CONTEXT_BIAS` into a Groq prompt
Edit point:
- Add a helper near `transcribe_audio_opus()` in `src/main.rs`

Recommended helper behavior:
- Input: raw env string like `"Claude Code,Clojure,Skia"`
- Split on commas
- Trim whitespace
- Drop empties
- Deduplicate while preserving order
- Build a concise prompt such as:
  - `"Use these spellings if relevant: Claude Code, Clojure, Skia."`
- If the resulting prompt is empty, omit `prompt`
- Cap length conservatively before request submission so we stay well below Groq's 224-token limit
  - simplest safe approach: truncate prompt string to a modest character budget, e.g. 300-500 chars

Notes:
- This preserves the old user-facing feature while adapting it to Groq's supported semantics.
- The prompt should remain advisory only; it must not include instructions unrelated to spelling/context.

#### 5. Update logging strings to reflect Groq
Edit points:
- `src/main.rs` log lines in `transcribe_audio_opus()`

Changes:
- `Sending Opus audio to Mistral API...` -> `Sending Ogg Opus audio to Groq Whisper API...`
- Any other Mistral/Voxtral-specific wording should be renamed to Groq/Whisper wording.

#### 6. Tighten error handling for non-2xx API responses
Edit point:
- `src/main.rs:501-509` request/response handling

Changes:
- Before `response.json()?`, call `error_for_status()?` or equivalent.
- This avoids trying to deserialize Groq error payloads into `TranscriptionResponse`.

Recommended patch shape:
- `let response = client.post(...).multipart(form).send()?.error_for_status()?;`

### Phase 2 — Align docs and configuration examples

#### 7. Update `CLAUDE.md`
Edit points:
- Project overview
- Environment setup
- Audio pipeline step 4

Changes:
- Replace Mistral/Voxtral references with Groq Whisper Large V3 Turbo
- Replace env vars:
  - `VOXTRAL_API_KEY` -> `GROQ_API_KEY`
  - `VOXTRAL_CONTEXT_BIAS` -> `CONTEXT_BIAS`
- Note that `CONTEXT_BIAS` is implemented through Groq Whisper's `prompt` guidance rather than a dedicated bias field

#### 8. Verify `.env.sample`
Edit point:
- `.env.sample`

Changes:
- Ensure the sample shows only current vars:
  - `GROQ_API_KEY=...`
  - optional `CONTEXT_BIAS=...`
- Remove any old Voxtral naming if still present later

### Phase 3 — Validation

#### 9. Static validation
Commands:
- `cargo check`
- `cargo fmt`
- `cargo clippy`

Expectation:
- No compile or lint regressions

#### 10. Runtime validation with a real request path
Validation goals:
- App still records and stops correctly
- Ogg Opus upload is accepted by Groq
- Transcript text still flows into `LAST_TRANSCRIPTION`
- Retype feature still works unchanged
- With `CONTEXT_BIAS` set to recognizable custom terms, transcription honors spelling better when relevant

Suggested validation method:
- Run app with `.env` containing `GROQ_API_KEY`
- Speak a short sentence including one or more bias terms from `CONTEXT_BIAS`
- Confirm transcript text prints and is typed into active window

If Groq rejects Ogg upload despite docs supporting `ogg`:
- Follow-up implementation option: add a temporary in-memory or temp-file WAV/FLAC conversion step before upload
- This is **not** planned as the primary path because the current Ogg container should already be compatible with the documented supported formats

---

## Exact patch spec

### `src/main.rs`
1. In `transcribe_audio_opus()`:
   - swap `VOXTRAL_API_KEY` -> `GROQ_API_KEY`
   - swap `VOXTRAL_CONTEXT_BIAS` -> `CONTEXT_BIAS`
   - change API URL to Groq transcription endpoint
   - change model to `whisper-large-v3-turbo`
   - keep/add `language = "en"`
   - add `response_format = "json"`
   - add `temperature = "0"`
   - rename upload file metadata to `audio.ogg` / `audio/ogg`
   - replace `context_bias` multipart field with `prompt`
   - call `error_for_status()` before JSON deserialization
   - update log strings
2. Add helper function, e.g.:
   - `fn build_groq_prompt(context_bias: &str) -> Option<String>`

### `CLAUDE.md`
- Rewrite Mistral/Voxtral references to Groq/Whisper
- Rewrite env var names
- Mention prompt-based context guidance

### `.env.sample`
- Keep only Groq-era variables and examples

---

## Risks / watchouts
- **Container vs extension mismatch:** current code labels Ogg Opus bytes as `.opus`; fixing this is important for provider compatibility.
- **Prompt size:** Groq limits prompt guidance to 224 tokens; keep `CONTEXT_BIAS` transformation concise.
- **Language hardcoding:** current code sets English. That is fine for this migration because the app already did so, but future work could make language configurable.
- **Error schema mismatch:** must not parse error bodies as success payloads.

## Non-goals for this change
- No provider abstraction layer
- No dual-provider fallback
- No translation endpoint support
- No language auto-detection/config UI changes
- No audio re-encoding rewrite unless Groq proves incompatible with Ogg Opus in practice

## Acceptance criteria
- No remaining runtime dependency on Mistral/Voxtral
- App reads `GROQ_API_KEY`
- Optional biasing reads `CONTEXT_BIAS`
- Biasing is passed as Groq `prompt`
- Requests go to Groq `whisper-large-v3-turbo`
- Documentation reflects the new provider and env vars
- Build/lint pass

## References
- Repo:
  - `src/main.rs`
  - `CLAUDE.md`
  - `.env.sample`
- Web:
  - https://console.groq.com/docs/speech-to-text
  - https://console.groq.com/docs/api-reference#audio-transcription
