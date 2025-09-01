use reqwest::blocking::multipart;
use serde::Deserialize;
use std::env;
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;
use chrono::Utc;

#[derive(Debug, Clone)]
enum AudioCommand {
    StartRecording,
    StopRecording,
}

struct AudioManager {
    recorder: AudioRecorder,
    current_stream: Option<cpal::Stream>,
}

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

struct AudioRecorder {
    recording: Arc<Mutex<bool>>,
    current_file: Arc<Mutex<Option<String>>>,
    writer: Arc<Mutex<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>,
}

impl Clone for AudioRecorder {
    fn clone(&self) -> Self {
        Self {
            recording: Arc::clone(&self.recording),
            current_file: Arc::clone(&self.current_file),
            writer: Arc::clone(&self.writer),
        }
    }
}

impl AudioRecorder {
    fn new() -> Self {
        Self {
            recording: Arc::new(Mutex::new(false)),
            current_file: Arc::new(Mutex::new(None)),
            writer: Arc::new(Mutex::new(None)),
        }
    }

    fn prepare_recording(&self) -> Result<String, String> {
        let mut recording = self.recording.lock().unwrap();
        if *recording {
            return Err("Already recording".to_string());
        }

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("recording_{}.wav", timestamp);
        
        let mut current_file = self.current_file.lock().unwrap();
        *current_file = Some(filename.clone());

        *recording = true;

        Ok(filename)
    }

    fn finalize_recording(&self) -> Result<Option<String>, String> {
        let mut recording = self.recording.lock().unwrap();
        if !*recording {
            return Ok(None);
        }

        *recording = false;

        // Finalize and close the WAV writer
        let mut writer_lock = self.writer.lock().unwrap();
        if let Some(writer) = writer_lock.take() {
            writer.finalize().map_err(|e| format!("Failed to finalize WAV file: {}", e))?;
        }
        drop(writer_lock);

        let mut current_file = self.current_file.lock().unwrap();
        let filename = current_file.take();

        if let Some(ref file) = filename {
            println!("Stopped recording: {}", file);
        }

        Ok(filename)
    }

    fn create_writer(&self, filename: &str) -> Result<(), String> {
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

        let wav_writer = hound::WavWriter::create(filename, spec)
            .map_err(|e| format!("Failed to create wav writer: {}", e))?;
        
        let mut writer_lock = self.writer.lock().unwrap();
        *writer_lock = Some(wav_writer);

        Ok(())
    }

    fn is_recording(&self) -> bool {
        *self.recording.lock().unwrap()
    }
}

impl AudioManager {
    fn new() -> Self {
        Self {
            recorder: AudioRecorder::new(),
            current_stream: None,
        }
    }

    fn start_recording(&mut self) -> Result<(), String> {
        if self.recorder.is_recording() {
            return Ok(());
        }

        let filename = self.recorder.prepare_recording()?;
        self.recorder.create_writer(&filename)?;

        println!("Started recording: {}", filename);

        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or("No input device available")?;

        let config = device.default_input_config().map_err(|e| format!("Failed to get input config: {}", e))?;
        
        let writer_arc = Arc::clone(&self.recorder.writer);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut writer_opt) = writer_arc.try_lock() {
                            if let Some(ref mut writer) = writer_opt.as_mut() {
                                for &sample in data {
                                    let sample_i16 = (sample * i16::MAX as f32) as i16;
                                    let _ = writer.write_sample(sample_i16);
                                }
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
        self.current_stream = Some(stream);

        Ok(())
    }

    fn stop_recording(&mut self) -> Result<(), String> {
        if !self.recorder.is_recording() {
            return Ok(());
        }

        // Stop the stream
        self.current_stream.take();

        // Finalize recording
        match self.recorder.finalize_recording()? {
            Some(filename) => {
                // Process transcription in background thread
                std::thread::spawn(move || {
                    println!("Processing transcription...");
                    if let Err(e) = transcribe_audio(&filename) {
                        eprintln!("Failed to transcribe audio: {}", e);
                    }
                    // Clean up the audio file after transcription
                    if let Err(e) = fs::remove_file(&filename) {
                        eprintln!("Warning: Failed to remove audio file {}: {}", filename, e);
                    }
                });
            }
            None => {}
        }

        Ok(())
    }
}

fn transcribe_audio(file_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    let api_key = env::var("MISTRAL_API_KEY")
        .expect("MISTRAL_API_KEY environment variable must be set");

    let audio_file = fs::read(file_path)
        .map_err(|e| format!("Failed to read audio file '{}': {}", file_path, e))?;

    let client = reqwest::blocking::Client::new();

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
        .send()?;

    let transcription: TranscriptionResponse = response.json()?;
    println!("Transcription: {}", transcription.text);

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new().map_err(|e| format!("Failed to create event loop: {}", e))?;
    let hotkey_manager = GlobalHotKeyManager::new().map_err(|e| format!("Failed to create hotkey manager: {}", e))?;

    let hotkey = HotKey::new(Some(Modifiers::ALT), Code::Period);
    hotkey_manager.register(hotkey).map_err(|e| format!("Failed to register hotkey: {}", e))?;

    let mut audio_manager = AudioManager::new();
    let (audio_tx, audio_rx) = flume::unbounded::<AudioCommand>();

    println!("Press and hold Right Alt + . to record audio");

    let result = event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        // Handle audio commands from channels
        if let Ok(command) = audio_rx.try_recv() {
            match command {
                AudioCommand::StartRecording => {
                    if let Err(e) = audio_manager.start_recording() {
                        eprintln!("Failed to start recording: {}", e);
                    }
                }
                AudioCommand::StopRecording => {
                    if let Err(e) = audio_manager.stop_recording() {
                        eprintln!("Failed to stop recording: {}", e);
                    }
                }
            }
        }

        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            match event.state {
                HotKeyState::Pressed => {
                    let tx = audio_tx.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = tx.send(AudioCommand::StartRecording) {
                            eprintln!("Failed to send start command: {}", e);
                        }
                    });
                }
                HotKeyState::Released => {
                    let tx = audio_tx.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = tx.send(AudioCommand::StopRecording) {
                            eprintln!("Failed to send stop command: {}", e);
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
