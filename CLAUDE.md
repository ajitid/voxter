# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview
This is a Rust project for speech-to-text functionality called "voxtral-speech-to-text". Currently in early development stage with minimal implementation.

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
The project is currently a basic Rust binary with:
- `src/main.rs` - Entry point with placeholder implementation
- `Cargo.toml` - Project configuration with no dependencies yet
- Standard Rust project structure expected to evolve as speech-to-text functionality is implemented