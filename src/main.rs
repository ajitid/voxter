use reqwest::blocking::multipart;
use serde::Deserialize;
use std::env;
use std::sync::Arc;
use std::sync::Mutex;


struct AudioManager {
    recorder: AudioRecorder,
    current_stream: Option<cpal::Stream>,
}

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::io::{self, Write};

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

struct AudioRecorder {
    recording: Arc<Mutex<bool>>,
    audio_buffer: Arc<Mutex<Vec<u8>>>,
    sample_rate: u32,
    channels: u16,
}

impl Clone for AudioRecorder {
    fn clone(&self) -> Self {
        Self {
            recording: Arc::clone(&self.recording),
            audio_buffer: Arc::clone(&self.audio_buffer),
            sample_rate: self.sample_rate,
            channels: self.channels,
        }
    }
}

impl AudioRecorder {
    fn new() -> Self {
        Self {
            recording: Arc::new(Mutex::new(false)),
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            sample_rate: 44100,
            channels: 1,
        }
    }

    fn create_wav_header(sample_rate: u32, channels: u16, data_size: u32) -> Vec<u8> {
        let mut header = Vec::with_capacity(44);
        
        // RIFF header
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&(36 + data_size).to_le_bytes());
        header.extend_from_slice(b"WAVE");
        
        // fmt chunk
        header.extend_from_slice(b"fmt ");
        header.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        header.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        header.extend_from_slice(&channels.to_le_bytes());
        header.extend_from_slice(&sample_rate.to_le_bytes());
        header.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes()); // byte rate
        header.extend_from_slice(&(channels * 2).to_le_bytes()); // block align
        header.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        
        // data chunk header
        header.extend_from_slice(b"data");
        header.extend_from_slice(&data_size.to_le_bytes());
        
        header
    }

    fn update_wav_header_size(buffer: &mut Vec<u8>, data_size: u32) {
        if buffer.len() >= 44 {
            // Update file size in RIFF header (bytes 4-7)
            let file_size = (36 + data_size).to_le_bytes();
            buffer[4..8].copy_from_slice(&file_size);
            
            // Update data size in data chunk header (bytes 40-43)
            let data_size_bytes = data_size.to_le_bytes();
            buffer[40..44].copy_from_slice(&data_size_bytes);
        }
    }

    fn prepare_recording(&self) -> Result<(), String> {
        let mut recording = self.recording.lock().unwrap();
        if *recording {
            return Err("Already recording".to_string());
        }

        // Initialize buffer with WAV header (placeholder for data size)
        let mut buffer = self.audio_buffer.lock().unwrap();
        buffer.clear();
        let header = Self::create_wav_header(self.sample_rate, self.channels, 0);
        buffer.extend_from_slice(&header);

        *recording = true;
        Ok(())
    }

    fn finalize_recording(&self) -> Result<bool, String> {
        let mut recording = self.recording.lock().unwrap();
        if !*recording {
            return Ok(false);
        }

        *recording = false;

        // Update WAV header with actual data size
        let mut buffer = self.audio_buffer.lock().unwrap();
        if buffer.len() > 44 {
            let data_size = (buffer.len() - 44) as u32;
            Self::update_wav_header_size(&mut buffer, data_size);
            println!("Stopped recording: {} bytes of audio data", data_size);
            Ok(true)
        } else {
            println!("No audio data recorded");
            Ok(false)
        }
    }

    fn configure_from_device(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or("No input device available")?;

        let config = device.default_input_config().map_err(|e| format!("Failed to get input config: {}", e))?;
        
        self.sample_rate = config.sample_rate().0;
        self.channels = config.channels();
        
        Ok(())
    }

    fn get_audio_buffer(&self) -> Vec<u8> {
        self.audio_buffer.lock().unwrap().clone()
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

        self.recorder.configure_from_device()?;
        self.recorder.prepare_recording()?;

        println!("Started recording ({}Hz, {} channels)", self.recorder.sample_rate, self.recorder.channels);

        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or("No input device available")?;

        let config = device.default_input_config().map_err(|e| format!("Failed to get input config: {}", e))?;
        
        let buffer_arc = Arc::clone(&self.recorder.audio_buffer);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buffer) = buffer_arc.try_lock() {
                            for &sample in data {
                                let sample_i16 = (sample * i16::MAX as f32) as i16;
                                buffer.extend_from_slice(&sample_i16.to_le_bytes());
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
        if self.recorder.finalize_recording()? {
            let audio_data = self.recorder.get_audio_buffer();
            // Process transcription in background thread
            std::thread::spawn(move || {
                println!("Processing transcription...");
                if let Err(e) = transcribe_audio(audio_data) {
                    eprintln!("Failed to transcribe audio: {}", e);
                }
            });
        }

        Ok(())
    }
}

fn transcribe_audio(audio_data: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    let api_key = env::var("MISTRAL_API_KEY")
        .expect("MISTRAL_API_KEY environment variable must be set");

    let client = reqwest::blocking::Client::new();

    let form = multipart::Form::new()
        .text("model", "voxtral-mini-latest")
        .text("language", "en")
        .part(
            "file",
            multipart::Part::bytes(audio_data)
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

    std::process::exit(0);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut audio_manager = AudioManager::new();

    println!("Recording started. Press Enter to stop and transcribe...");
    
    // Start recording immediately
    if let Err(e) = audio_manager.start_recording() {
        eprintln!("Failed to start recording: {}", e);
        return Err(e.into());
    }

    // Wait for Enter key
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    // Stop recording and process transcription
    if let Err(e) = audio_manager.stop_recording() {
        eprintln!("Failed to stop recording: {}", e);
        return Err(e.into());
    }

    // Keep the main thread alive to allow transcription to complete
    // The transcription function will exit the program when done
    std::thread::park();

    Ok(())
}
