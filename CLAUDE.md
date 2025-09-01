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
The application is structured around manual audio recording triggered by user input:

### Core Components
- `AudioManager` - Manages recording state and audio streams
- `AudioRecorder` - Handles WAV file creation and audio data writing
- Manual trigger system using stdin (press Enter to stop recording)
- Channel-based communication between components using Arc<Mutex<T>>

### Audio Processing Flow
1. Application starts → Automatically begins recording to timestamped WAV file
2. Audio data captured via `cpal` and written using `hound`
3. User presses Enter → Stop recording and trigger transcription
4. Background thread sends audio to Mistral API for transcription
5. Audio file cleaned up after successful transcription
6. Application exits after transcription completes

### External Dependencies
- Requires `MISTRAL_API_KEY` environment variable for transcription
- Uses Mistral's `voxtral-mini-latest` model
- Audio format: 16-bit PCM WAV files with automatic device configuration
- Uses `dotenvy` for loading environment variables from .env files