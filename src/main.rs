use reqwest::multipart;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = env::var("MISTRAL_API_KEY")
        .expect("MISTRAL_API_KEY environment variable must be set");

    let audio_file_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "audio.mp3".to_string());

    let audio_file = fs::read(&audio_file_path)
        .map_err(|e| format!("Failed to read audio file '{}': {}", audio_file_path, e))?;

    let client = reqwest::Client::new();

    let form = multipart::Form::new()
        .text("model", "voxtral-mini-latest")
        .text("language", "en")
        .part(
            "file",
            multipart::Part::bytes(audio_file)
                .file_name("audio.mp3")
                .mime_str("audio/mpeg")?,
        );

    let response = client
        .post("https://api.mistral.ai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await?;

    let transcription: TranscriptionResponse = response.json().await?;

    println!("{}", transcription.text);

    Ok(())
}
