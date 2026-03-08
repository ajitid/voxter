# Todo: migrate to Groq whisper-large-v3-turbo

## Phase 1 — inspect and prep
- [x] Verify current transcription integration points in `src/main.rs`
- [x] Verify Groq Whisper transcription request payload from web docs
- [x] Verify whether Groq supports prompt/context guidance
- [x] Confirm whether this should be a clean breaking change

## Phase 2 — code changes
- [x] Replace Mistral/Voxtral API usage with Groq transcription endpoint
- [x] Rename env vars to `GROQ_API_KEY` and `CONTEXT_BIAS`
- [x] Map `CONTEXT_BIAS` to Groq `prompt`
- [x] Update upload metadata from `.opus` / `audio/opus` to `.ogg` / `audio/ogg`
- [x] Improve non-2xx response handling
- [x] Update provider-specific logging strings

## Phase 3 — docs/config
- [x] Update `CLAUDE.md` to document Groq Whisper usage
- [x] Verify `.env.sample` matches the new env vars

## Phase 4 — verification
- [x] Run `cargo check`
- [x] Run `cargo fmt`
- [x] Run `cargo clippy`
- [ ] Smoke-test transcription flow with `GROQ_API_KEY`
- [ ] Smoke-test spelling guidance via `CONTEXT_BIAS`
