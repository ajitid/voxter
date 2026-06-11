# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview
A Rust desktop application that records audio via global hotkeys and transcribes it using Groq's Whisper Large V3 Turbo API. Transcriptions are automatically typed into the active window.

## Development Commands
- `cargo build` - Build the project
- `cargo run` - Build and run the application
- `cargo check` - Quick syntax and type checking
- `cargo fmt` - Format code
- `cargo clippy` - Run linter
- Run `cargo fmt && cargo clippy` before committing (Rust 2024 edition)

## Environment Setup
- Requires `VOXTER_API_KEY` environment variable (loaded via `.env` file with dotenvy)
- Optional `VOXTER_CONTEXT_BIAS` for domain-specific vocabulary / preferred spellings (comma-separated words or phrases, passed to Mistral Voxtral as `context_bias`)
- On macOS: App needs Accessibility permissions for `enigo` keyboard simulation and `rdev` global hotkey capture

## Architecture

### Recording Modes
- **HOLD mode**: Hold Right Option/Alt to record, release to transcribe
- **LATCH mode**: Press Space during HOLD to switch; press the hotkey again to stop recording
- **Menu bar**: Use the menu-bar microphone icon to type the last transcription or quit

### Core Components (all in `src/main.rs`)
- `AudioManager` - Orchestrates recording lifecycle, owns `cpal::Stream` (not Send/Sync, must stay on main thread)
- `AudioRecorder` - Manages recording state with `Arc<Mutex<T>>` for thread-safe access
- `run_opus_worker()` - Dedicated thread for real-time Opus encoding into Ogg container
- `check_speech_activity()` - VAD (Voice Activity Detection) to skip transcription when no speech detected

### Audio Pipeline
1. `cpal` captures audio (f32 samples at device sample rate)
2. Audio callback downmixes to mono, converts f32→i16, sends via `flume` channel
3. Opus worker thread encodes 20ms frames (24kbps) into Ogg container
4. On stop: finalize Ogg stream, run VAD check, send to Groq Whisper API if speech detected
5. Transcription auto-typed via `enigo`, audio feedback via `rodio` (assets/on.mp3, assets/off.mp3)

### Threading Model
- Main thread: Runs `AudioManager`, handles control messages via `mpsc` channel
- rdev thread: Global hotkey listener, sends `ControlMsg` to main thread
- Opus worker thread: Spawned per recording session
- Transcription thread: Spawned after recording stops for API call + typing

### Key Implementation Details
- Recordings under 0.9s are skipped (too short for meaningful transcription)
- VAD uses `voice_activity_detector` crate with 16kHz downsampled audio
- Audio feedback sounds play with 100ms silence prefix to avoid audio system wake-up cut-off
