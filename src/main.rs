use reqwest::blocking::multipart;
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::io;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

struct AudioManager {
    recorder: AudioRecorder,
    current_stream: Option<cpal::Stream>,
}

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

struct AudioRecorder {
    recording: Arc<Mutex<bool>>,
    sample_rate: u32,
    channels: u16,
    tx: Arc<Mutex<Option<flume::Sender<Vec<i16>>>>>,
    result_rx: Arc<Mutex<Option<flume::Receiver<Vec<u8>>>>>,
}

impl Clone for AudioRecorder {
    fn clone(&self) -> Self {
        Self {
            recording: Arc::clone(&self.recording),
            sample_rate: self.sample_rate,
            channels: self.channels,
            tx: Arc::clone(&self.tx),
            result_rx: Arc::clone(&self.result_rx),
        }
    }
}

impl AudioRecorder {
    fn new() -> Self {
        Self {
            recording: Arc::new(Mutex::new(false)),
            sample_rate: 16000,
            channels: 1,
            tx: Arc::new(Mutex::new(None)),
            result_rx: Arc::new(Mutex::new(None)),
        }
    }

    // WAV buffer helpers removed; streaming Opus is used instead.

    fn prepare_recording(&self) -> Result<(), String> {
        let mut recording = self.recording.lock().unwrap();
        if *recording {
            return Err("Already recording".to_string());
        }

        // Initialize streaming Opus worker and channels
        let (tx, rx) = flume::bounded::<Vec<i16>>(8);
        let (result_tx, result_rx) = flume::bounded::<Vec<u8>>(1);

        *self.tx.lock().unwrap() = Some(tx);
        *self.result_rx.lock().unwrap() = Some(result_rx);

        let sr = self.sample_rate;
        std::thread::spawn(move || {
            if let Err(e) = run_opus_worker(rx, result_tx, sr) {
                eprintln!("Opus worker error: {}", e);
            }
        });

        *recording = true;
        Ok(())
    }

    fn finalize_recording(&self) -> Result<bool, String> {
        let mut recording = self.recording.lock().unwrap();
        if !*recording {
            return Ok(false);
        }

        *recording = false;

        Ok(true)
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

    // Removed WAV buffer and on-stop conversion. Streaming Opus is used instead.

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

        let tx_arc = Arc::clone(&self.recorder.tx);

        let channels_cfg = config.channels() as usize;
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        // Downmix to mono if needed and convert to i16
                        let chunk: Vec<i16> = if channels_cfg > 1 {
                            let frames = data.len() / channels_cfg;
                            let mut mono = Vec::with_capacity(frames);
                            for i in 0..frames {
                                let mut acc = 0.0f32;
                                let base = i * channels_cfg;
                                for c in 0..channels_cfg {
                                    acc += data[base + c];
                                }
                                let avg = acc / (channels_cfg as f32);
                                let clamped = avg.max(-1.0).min(1.0);
                                mono.push((clamped * (i16::MAX as f32)) as i16);
                            }
                            mono
                        } else {
                            let mut mono = Vec::with_capacity(data.len());
                            for &s in data {
                                let clamped = s.max(-1.0).min(1.0);
                                mono.push((clamped * (i16::MAX as f32)) as i16);
                            }
                            mono
                        };
                        if let Ok(guard) = tx_arc.lock() {
                            if let Some(tx) = &*guard {
                                let _ = tx.try_send(chunk);
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
            // Close the sender to signal worker end-of-stream
            if let Ok(mut guard) = self.recorder.tx.lock() {
                guard.take();
            }
            let result_rx_arc = Arc::clone(&self.recorder.result_rx);
            std::thread::spawn(move || {
                println!("Finalizing Opus stream...");
                let start = Instant::now();
                let opus_data = {
                    let mut guard = result_rx_arc.lock().unwrap();
                    match guard.take().unwrap().recv() {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            eprintln!("Failed to receive Opus data: {}", e);
                            Vec::new()
                        }
                    }
                };
                let dt = start.elapsed();
                println!("Opus finalize time: {:.3} ms", dt.as_secs_f64() * 1000.0);
                if !opus_data.is_empty() {
                    /*
                    // Save Opus to file before transcription
                    if let Err(e) = save_opus_file(&opus_data) {
                        eprintln!("Failed to save Opus file: {}", e);
                    }
                    */
                    println!("Processing transcription...");
                    if let Err(e) = transcribe_audio_opus(opus_data) {
                        eprintln!("Failed to transcribe audio: {}", e);
                    }
                } else {
                    eprintln!("No Opus data produced");
                }
            });
        }

        Ok(())
    }
}

fn _save_opus_file(opus_data: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let filename = format!("recording_{}.opus", ts);
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

fn run_opus_worker(
    rx: flume::Receiver<Vec<i16>>,
    result_tx: flume::Sender<Vec<u8>>,
    sample_rate: u32,
) -> Result<(), String> {
    // Configure Opus encoder for mono speech
    let mut encoder =
        opus::Encoder::new(sample_rate, opus::Channels::Mono, opus::Application::Voip)
            .map_err(|e| format!("Failed to create Opus encoder: {}", e))?;
    encoder
        .set_bitrate(opus::Bitrate::Bits(24000))
        .map_err(|e| format!("Failed to set bitrate: {}", e))?;

    // Prepare Ogg container
    let mut ogg_data = Vec::new();
    let mut writer = ogg::PacketWriter::new(&mut ogg_data);
    let serial_number: u32 = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
        ^ 0xA5A5_5A5A_F00D_F00D) as u32;

    // Identification header
    let mut opus_head = Vec::new();
    opus_head.extend_from_slice(b"OpusHead");
    opus_head.push(1);
    opus_head.push(1u8); // mono
    opus_head.extend_from_slice(&0u16.to_le_bytes());
    opus_head.extend_from_slice(&sample_rate.to_le_bytes());
    opus_head.extend_from_slice(&0u16.to_le_bytes());
    opus_head.push(0);
    writer
        .write_packet(
            opus_head,
            serial_number,
            ogg::PacketWriteEndInfo::EndPage,
            0u64,
        )
        .map_err(|e| format!("Failed to write Opus header: {}", e))?;

    // Comment header
    let mut opus_tags = Vec::new();
    opus_tags.extend_from_slice(b"OpusTags");
    let vendor = b"voxtral-speech-to-text";
    opus_tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    opus_tags.extend_from_slice(vendor);
    opus_tags.extend_from_slice(&0u32.to_le_bytes());
    writer
        .write_packet(
            opus_tags,
            serial_number,
            ogg::PacketWriteEndInfo::EndPage,
            0u64,
        )
        .map_err(|e| format!("Failed to write Opus comments: {}", e))?;

    // Stream frames
    let frame_size: usize = (sample_rate as usize) / 50; // 20ms frames
    let mut out_buf = [0u8; 4000];
    let mut granulepos: u64 = 0;
    let mut accum: Vec<i16> = Vec::with_capacity(frame_size * 2);
    while let Ok(mut chunk) = rx.recv() {
        accum.append(&mut chunk);
        while accum.len() >= frame_size {
            let frame: Vec<i16> = accum.drain(..frame_size).collect();
            match encoder.encode(&frame, &mut out_buf) {
                Ok(len) => {
                    let packet = out_buf[..len].to_vec();
                    granulepos = granulepos
                        .saturating_add((frame_size as u64) * 48_000u64 / (sample_rate as u64));
                    writer
                        .write_packet(
                            packet,
                            serial_number,
                            ogg::PacketWriteEndInfo::NormalPacket,
                            granulepos,
                        )
                        .map_err(|e| format!("Failed to write Opus packet: {}", e))?;
                }
                Err(e) => return Err(format!("Opus encoding error: {}", e)),
            }
        }
    }

    // Flush remaining samples (pad to full frame)
    if !accum.is_empty() {
        let mut frame = vec![0i16; frame_size];
        for (i, &s) in accum.iter().enumerate() {
            if i < frame_size {
                frame[i] = s;
            } else {
                break;
            }
        }
        if let Ok(len) = encoder.encode(&frame, &mut out_buf) {
            let packet = out_buf[..len].to_vec();
            granulepos =
                granulepos.saturating_add((frame_size as u64) * 48_000u64 / (sample_rate as u64));
            writer
                .write_packet(
                    packet,
                    serial_number,
                    ogg::PacketWriteEndInfo::NormalPacket,
                    granulepos,
                )
                .map_err(|e| format!("Failed to write Opus packet: {}", e))?;
        }
    }

    // End stream
    writer
        .write_packet(
            Vec::new(),
            serial_number,
            ogg::PacketWriteEndInfo::EndStream,
            granulepos,
        )
        .map_err(|e| format!("Failed to finalize Opus stream: {}", e))?;

    let _ = result_tx.send(ogg_data);
    Ok(())
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
