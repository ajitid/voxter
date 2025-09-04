# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview
This is a Rust project for speech-to-text functionality called "voxtral-speech-to-text". It's a desktop application that records audio via manual input and transcribes it using Mistral's Voxtral API.

## Development Commands

### Build and Run
- `cargo build` - Build the project
- `cargo run` - Build and run the application
- `cargo check` - Quick syntax and type checking without building
- `cargo test` - Run tests
- `cargo fmt` - Format code according to Rust standards
- `cargo clippy` - Run the Clippy linter for additional checks

### Development Workflow
- Use `cargo check` for fast feedback during development
- Run `cargo fmt` and `cargo clippy` before committing changes
- The project uses Rust 2024 edition

## Architecture
The application is structured around global hotkey-triggered audio recording:

### Core Components
- `AudioManager` - Manages recording state and audio streams  
- `AudioRecorder` - Handles Opus encoding and audio data processing
- Global hotkey system using `rdev` (Right Alt/AltGr or Right Command keys)
- Channel-based communication between components using Arc<Mutex<T>> and flume channels
- Real-time Opus encoding worker thread with Ogg container format

### Audio Processing Flow
1. Application starts → Listens for global hotkey press
2. Hotkey pressed → Begin recording with real-time Opus encoding
3. Audio data captured via `cpal` with f32 to i16 PCM conversion and mono downmixing
4. Streaming Opus encoder processes audio in 20ms frames within Ogg container
5. Hotkey released → Stop recording and finalize Opus stream
6. Opus audio sent directly to Mistral API for transcription
7. Transcribed text automatically typed into active window using `enigo`
8. Audio feedback played via `rodio` (on.mp3/off.mp3 sound cues)

### External Dependencies
- Requires `MISTRAL_API_KEY` environment variable for transcription
- Uses Mistral's `voxtral-mini-latest` model  
- Audio format: Real-time Opus encoding in Ogg container (24kbps bitrate)
- Global hotkey detection via `rdev` library (platform-dependent permissions may apply)
- Text input simulation via `enigo` for typing transcriptions into active windows
- Audio playback via `rodio` for user feedback sounds
- Uses `dotenvy` for loading environment variables from .env files
- API response time measurement and automatic text typing included
