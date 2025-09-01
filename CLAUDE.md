# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview
This is a Rust project for speech-to-text functionality called "voxtral-speech-to-text". It's a desktop application that records audio via global hotkeys and transcribes it using Mistral's Voxtral API.

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
The application is structured around event-driven audio recording:

### Core Components
- `AudioManager` - Manages recording state and audio streams
- `AudioRecorder` - Handles WAV file creation and audio data writing
- Global hotkey system using `global-hotkey` crate (Alt + Period)
- Event loop using `winit` for handling UI events and hotkey state
- Channel-based communication between hotkey events and audio operations

### Audio Processing Flow
1. Hotkey pressed → Start recording to timestamped WAV file
2. Audio data captured via `cpal` and written using `hound`
3. Hotkey released → Stop recording and trigger transcription
4. Background thread sends audio to Mistral API for transcription
5. Audio file cleaned up after successful transcription

### External Dependencies
- Requires `MISTRAL_API_KEY` environment variable for transcription
- Uses Mistral's `voxtral-mini-latest` model
- Audio format: 16-bit PCM WAV files