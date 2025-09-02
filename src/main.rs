use reqwest::blocking::multipart;
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

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
            sample_rate: 16000,
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
            // The modification converts bytes to megabytes by dividing by 1,048,576 (1024²)
            println!(
                "Stopped recording: {:.2} MB of audio data",
                data_size as f64 / 1_048_576.0
            );
            Ok(true)
        } else {
            println!("No audio data recorded");
            Ok(false)
        }
    }

    fn configure_from_device(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        self.sample_rate = config.sample_rate().0;
        self.channels = config.channels();

        Ok(())
    }

    fn get_audio_buffer(&self) -> Vec<u8> {
        self.audio_buffer.lock().unwrap().clone()
    }

    fn convert_to_opus(&self) -> Result<Vec<u8>, String> {
        let buffer = self.audio_buffer.lock().unwrap();
        if buffer.len() <= 44 {
            return Err("No audio data to convert".to_string());
        }

        // Extract PCM data (skip WAV header)
        let pcm_data = &buffer[44..];

        // Convert bytes back to i16 samples (interleaved if channels > 1)
        let raw_samples: Vec<i16> = pcm_data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        // Downmix to mono if needed to match Opus encoder settings
        let mono_samples: Vec<i16> = if self.channels > 1 {
            let ch = self.channels as usize;
            raw_samples
                .chunks_exact(ch)
                .map(|frame| {
                    let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                    (sum / ch as i32) as i16
                })
                .collect()
        } else {
            raw_samples
        };

        // Configure Opus encoder for speech
        let mut encoder = opus::Encoder::new(
            self.sample_rate,
            opus::Channels::Mono,
            opus::Application::Voip,
        )
        .map_err(|e| format!("Failed to create Opus encoder: {}", e))?;

        // Set bitrate for speech (24 kbps is good for speech quality)
        encoder
            .set_bitrate(opus::Bitrate::Bits(24000))
            .map_err(|e| format!("Failed to set bitrate: {}", e))?;

        // Create Ogg container for Opus data
        let mut ogg_data = Vec::new();
        let mut writer = ogg::PacketWriter::new(&mut ogg_data);
        // Use a per-file serial number (simple time-based mix to avoid collisions)
        let serial_number: u32 = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
            ^ 0xA5A5_5A5A_F00D_F00D) as u32;

        // Create Opus identification header
        let mut opus_head = Vec::new();
        opus_head.extend_from_slice(b"OpusHead");
        opus_head.push(1); // version
        // We encode mono to keep things simple and robust
        opus_head.push(1u8); // channel count
        opus_head.extend_from_slice(&0u16.to_le_bytes()); // pre-skip
        opus_head.extend_from_slice(&self.sample_rate.to_le_bytes()); // original sample rate
        opus_head.extend_from_slice(&0u16.to_le_bytes()); // output gain
        opus_head.push(0); // channel mapping family

        // Write identification header as first page (separate page, gp=0)
        writer
            .write_packet(
                opus_head,
                serial_number,
                ogg::PacketWriteEndInfo::EndPage,
                0u64,
            )
            .map_err(|e| format!("Failed to write Opus header: {}", e))?;

        // Create Opus comment header
        let mut opus_tags = Vec::new();
        opus_tags.extend_from_slice(b"OpusTags");
        let vendor = b"voxtral-speech-to-text";
        opus_tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        opus_tags.extend_from_slice(vendor);
        opus_tags.extend_from_slice(&0u32.to_le_bytes()); // user comment list length

        // Write comment header as second page (separate page, gp=0)
        writer
            .write_packet(
                opus_tags,
                serial_number,
                ogg::PacketWriteEndInfo::EndPage,
                0u64,
            )
            .map_err(|e| format!("Failed to write Opus comments: {}", e))?;

        // Encode audio in chunks (Opus works with fixed frame sizes)
        // Choose 20ms frames based on current sample rate
        let frame_size: usize = (self.sample_rate as usize) / 50; // 20ms
        let mut output_buffer = [0u8; 4000]; // Max Opus packet size
        // Track absolute granule position in 48kHz decoded samples
        let mut granulepos: u64 = 0;
        let total_frames = (mono_samples.len() + frame_size - 1) / frame_size;

        for (frame_index, chunk) in mono_samples.chunks(frame_size).enumerate() {
            // Pad the last chunk if necessary
            let mut frame = vec![0i16; frame_size];
            for (i, &sample) in chunk.iter().enumerate() {
                frame[i] = sample;
            }

            match encoder.encode(&frame, &mut output_buffer) {
                Ok(len) => {
                    let packet_data = output_buffer[..len].to_vec();
                    // Advance granule position by the decoded duration at 48kHz
                    granulepos = granulepos.saturating_add(
                        (frame_size as u64) * 48_000u64 / (self.sample_rate as u64),
                    );

                    let end_info = if frame_index + 1 == total_frames {
                        ogg::PacketWriteEndInfo::EndStream
                    } else {
                        ogg::PacketWriteEndInfo::NormalPacket
                    };

                    writer
                        .write_packet(packet_data, serial_number, end_info, granulepos as u64)
                        .map_err(|e| format!("Failed to write Opus packet: {}", e))?;
                }
                Err(e) => return Err(format!("Opus encoding error: {}", e)),
            }
        }

        Ok(ogg_data)
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

        println!(
            "Started recording ({}Hz, {} channels)",
            self.recorder.sample_rate, self.recorder.channels
        );

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        let buffer_arc = Arc::clone(&self.recorder.audio_buffer);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
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
                )
                .map_err(|e| format!("Failed to build input stream: {}", e))?,
            _ => return Err("Unsupported sample format".into()),
        };

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;
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
            let recorder_clone = self.recorder.clone();
            // Process transcription in background thread
            std::thread::spawn(move || {
                // Generate timestamp for consistent file naming
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                // Save the original WAV file first
                let wav_data = recorder_clone.get_audio_buffer();
                if let Err(e) = save_wav_file(&wav_data, timestamp) {
                    eprintln!("Failed to save WAV file: {}", e);
                }

                println!("Converting to Opus format...");
                let conversion_start = Instant::now();
                match recorder_clone.convert_to_opus() {
                    Ok(opus_data) => {
                        let conversion_latency = conversion_start.elapsed();
                        println!(
                            "Opus Conversion Time: {:.2}ms",
                            conversion_latency.as_millis()
                        );
                        let original_size = wav_data.len();
                        println!(
                            "Compression: {:.1}% ({}KB → {}KB)",
                            100.0 - (opus_data.len() as f64 / original_size as f64 * 100.0),
                            original_size / 1024,
                            opus_data.len() / 1024
                        );

                        // Save the Opus file
                        if let Err(e) = save_opus_file(&opus_data, timestamp) {
                            eprintln!("Failed to save Opus file: {}", e);
                        }

                        println!("Processing transcription...");
                        if let Err(e) = transcribe_audio_opus(opus_data) {
                            eprintln!("Failed to transcribe audio: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to convert to Opus: {}", e);
                    }
                }
            });
        }

        Ok(())
    }
}

fn save_wav_file(wav_data: &[u8], timestamp: u64) -> Result<String, Box<dyn std::error::Error>> {
    let filename = format!("recording_{}.wav", timestamp);
    let mut file = File::create(&filename)?;
    file.write_all(wav_data)?;

    println!("Saved WAV audio to: {}", filename);
    Ok(filename)
}

fn save_opus_file(opus_data: &[u8], timestamp: u64) -> Result<String, Box<dyn std::error::Error>> {
    let filename = format!("recording_{}.opus", timestamp);
    let mut file = File::create(&filename)?;
    file.write_all(opus_data)?;

    println!("Saved Opus audio to: {}", filename);
    Ok(filename)
}

fn transcribe_audio_opus(opus_data: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let api_key =
        env::var("MISTRAL_API_KEY").expect("MISTRAL_API_KEY environment variable must be set");

    let client = reqwest::blocking::Client::new();

    let form = multipart::Form::new()
        .text("model", "voxtral-mini-latest")
        .text("language", "en")
        .part(
            "file",
            multipart::Part::bytes(opus_data)
                .file_name("audio.opus")
                .mime_str("audio/opus")?,
        );

    println!("Sending Opus audio to Mistral API...");
    let start_time = Instant::now();

    let response = client
        .post("https://api.mistral.ai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()?;

    let api_latency = start_time.elapsed();
    let transcription: TranscriptionResponse = response.json()?;

    println!("API Response Time: {:.2}ms", api_latency.as_millis());
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
