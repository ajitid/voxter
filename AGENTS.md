# Repository Guidelines

## Project Structure & Modules
- Root: `Cargo.toml` defines a single binary crate.
- Source: `src/main.rs` implements recording, Opus/Ogg encoding, and API calls.
- Artifacts: recordings saved as `recording_<timestamp>.wav` and `.opus` in project root.
- Config: `.env` for secrets (e.g., `MISTRAL_API_KEY`).

## Build, Run, and Dev Commands
- Build: `cargo build` (use `--release` for optimized binary).
- Run: `cargo run` then press Enter to stop recording and transcribe.
- Lint: `cargo clippy --all-targets -- -D warnings`.
- Format: `cargo fmt --all`.
- Clean: `cargo clean`.

Notes (Windows/Build deps): The Opus bindings compile native code; ensure a C toolchain and CMake are available (e.g., Visual Studio Build Tools + CMake).

## Coding Style & Naming
- Rustfmt: required; 4‑space indentation, max line length per default toolchain.
- Naming: modules/files `snake_case`; functions/vars `snake_case`; types/enums `CamelCase`; constants `SCREAMING_SNAKE_CASE`.
- Imports: group std/crate/third‑party; prefer explicit over glob imports.

## Testing Guidelines
- Framework: Rust `cargo test` (unit/integration). No tests exist yet; add new tests under `tests/` or inline `#[cfg(test)]` blocks.
- Coverage: keep critical paths (audio buffering, Opus conversion, request building) covered. Aim for tests that mock I/O and network.
- Naming: `tests/<feature>_test.rs`; function names describe behavior, e.g., `encodes_opus_headers`.

## Commit & Pull Requests
- Commit style: short, imperative, and focused (observed: “add X”, “fix Y”, “wip …”).
- Scope: one logical change per commit; include rationale when non‑obvious.
- PRs must include: concise description, motivation, before/after behavior, test notes, and any logs/screenshots relevant to CLI output. Link related issues.

## Security & Configuration
- Secrets: set `MISTRAL_API_KEY` in `.env` or environment. Do not commit secrets.
- Audio permissions: ensure microphone access on your OS.
- Networking: endpoint `https://api.mistral.ai/v1/audio/transcriptions` (blocking client).

## Architecture Overview
- Flow: capture PCM via `cpal` → buffer WAV (header + PCM) → encode to Opus inside Ogg → save files → POST multipart to Mistral → print latency and transcript.
- Performance: use `--release` for lower latency and smaller binaries.
