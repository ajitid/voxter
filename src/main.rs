use reqwest::multipart;
use serde::Deserialize;
use std::env;
use std::fs;
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::Utc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use global_hotkey::{GlobalHotKeyManager, HotKeyState, hotkey::{HotKey, Modifiers, Code}, GlobalHotKeyEvent};
use winit::{
    event_loop::{ControlFlow, EventLoop},
    event::{Event, WindowEvent},
};

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

#[derive(Clone)]
struct AudioRecorder {
    recording: Arc<Mutex<bool>>,
    current_file: Arc<Mutex<Option<String>>>,
}

impl AudioRecorder {
    fn new() -> Self {
        Self {
            recording: Arc::new(Mutex::new(false)),
            current_file: Arc::new(Mutex::new(None)),
        }
    }

    async fn start_recording(&self) -> Result<(), String> {
        let mut recording = self.recording.lock().await;
        if *recording {
            return Ok(());
        }

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("recording_{}.wav", timestamp);
        
        let mut current_file = self.current_file.lock().await;
        *current_file = Some(filename.clone());
        *recording = true;

        println!("Started recording: {}", filename);

        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or("No input device available")?;

        let config = device.default_input_config().map_err(|e| format!("Failed to get input config: {}", e))?;
        
        let spec = hound::WavSpec {
            channels: config.channels(),
            sample_rate: config.sample_rate().0,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = Arc::new(Mutex::new(hound::WavWriter::create(&filename, spec).map_err(|e| format!("Failed to create wav writer: {}", e))?));
        let recording_clone = self.recording.clone();

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut writer) = writer.try_lock() {
                            for &sample in data {
                                let sample_i16 = (sample * i16::MAX as f32) as i16;
                                let _ = writer.write_sample(sample_i16);
                            }
                        }
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                ).map_err(|e| format!("Failed to build input stream: {}", e))?
            }
            _ => return Err("Unsupported sample format".into()),
        };

        stream.play().map_err(|e| format!("Failed to start stream: {}", e))?;
        
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if !*recording_clone.lock().await {
                    break;
                }
            }
        });

        Ok(())
    }

    async fn stop_recording(&self) -> Result<Option<String>, String> {
        let mut recording = self.recording.lock().await;
        if !*recording {
            return Ok(None);
        }

        *recording = false;
        let mut current_file = self.current_file.lock().await;
        let filename = current_file.take();

        if let Some(ref file) = filename {
            println!("Stopped recording: {}", file);
        }

        Ok(filename)
    }
}

async fn transcribe_audio(file_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    let api_key = env::var("MISTRAL_API_KEY")
        .expect("MISTRAL_API_KEY environment variable must be set");

    let audio_file = fs::read(file_path)
        .map_err(|e| format!("Failed to read audio file '{}': {}", file_path, e))?;

    let client = reqwest::Client::new();

    let form = multipart::Form::new()
        .text("model", "voxtral-mini-latest")
        .text("language", "en")
        .part(
            "file",
            multipart::Part::bytes(audio_file)
                .file_name("audio.wav")
                .mime_str("audio/wav")?,
        );

    let response = client
        .post("https://api.mistral.ai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await?;

    let transcription: TranscriptionResponse = response.json().await?;
    println!("Transcription: {}", transcription.text);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new().map_err(|e| format!("Failed to create event loop: {}", e))?;
    let hotkey_manager = GlobalHotKeyManager::new().map_err(|e| format!("Failed to create hotkey manager: {}", e))?;

    let hotkey = HotKey::new(Some(Modifiers::ALT), Code::Period);
    hotkey_manager.register(hotkey).map_err(|e| format!("Failed to register hotkey: {}", e))?;

    let recorder = AudioRecorder::new();

    println!("Press and hold Right Alt + . to record audio");

    let result = event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            match event.state {
                HotKeyState::Pressed => {
                    let recorder_clone = recorder.clone();
                    tokio::spawn(async move {
                        if let Err(e) = recorder_clone.start_recording().await {
                            eprintln!("Failed to start recording: {}", e);
                        }
                    });
                }
                HotKeyState::Released => {
                    let recorder_clone = recorder.clone();
                    tokio::spawn(async move {
                        match recorder_clone.stop_recording().await {
                            Ok(Some(filename)) => {
                                if let Err(e) = transcribe_audio(&filename).await {
                                    eprintln!("Failed to transcribe audio: {}", e);
                                }
                            }
                            Ok(None) => {}
                            Err(e) => eprintln!("Failed to stop recording: {}", e),
                        }
                    });
                }
            }
        }

        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                elwt.exit();
            }
            _ => {}
        }
    });

    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Event loop error: {}", e).into()),
    }
}
