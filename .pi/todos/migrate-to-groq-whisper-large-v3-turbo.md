# Todo: migrate to Groq whisper-large-v3-turbo

## Phase 1 — inspect and prep
- [x] Verify current transcription integration points in `src/main.rs`
- [x] Verify Groq Whisper transcription request payload from web docs
- [x] Verify whether Groq supports prompt/context guidance
- [x] Confirm whether this should be a clean breaking change

## Phase 2 — code changes
- [ ] Replace Mistral/Voxtral API usage with Groq transcription endpoint
- [ ] Rename env vars to `GROQ_API_KEY` and `CONTEXT_BIAS`
- [ ] Map `CONTEXT_BIAS` to Groq `prompt`
- [ ] Update upload metadata from `.opus` / `audio/opus` to `.ogg` / `audio/ogg`
- [ ] Improve non-2xx response handling
- [ ] Update provider-specific logging strings

## Phase 3 — docs/config
- [ ] Update `CLAUDE.md` to document Groq Whisper usage
- [ ] Verify `.env.sample` matches the new env vars

## Phase 4 — verification
- [ ] Run `cargo check`
- [ ] Run `cargo fmt`
- [ ] Run `cargo clippy`
- [ ] Smoke-test transcription flow with `GROQ_API_KEY`
- [ ] Smoke-test spelling guidance via `CONTEXT_BIAS`
