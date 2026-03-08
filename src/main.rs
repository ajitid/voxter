use reqwest::blocking::multipart;
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;
use voice_activity_detector::VoiceActivityDetector;

// Global state for storing the last transcription
static LAST_TRANSCRIPTION: std::sync::OnceLock<Arc<Mutex<Option<String>>>> =
    std::sync::OnceLock::new();

fn play_sound<P: AsRef<std::path::Path>>(path: P) {
    let path_buf = path.as_ref().to_path_buf();
    thread::spawn(move || {
        use rodio::Source;
        use rodio::stream::OutputStreamBuilder;
        use std::time::Duration;

        let mut stream_handle = match OutputStreamBuilder::open_default_stream() {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Failed to open default output stream: {}", e);
                return;
            }
        };
        // Optional: avoid noisy drop log in release; keep during dev if needed
        stream_handle.log_on_drop(false);

        let mixer = stream_handle.mixer();
        let sink = rodio::Sink::connect_new(mixer);

        match std::fs::File::open(&path_buf) {
            Ok(file) => {
                let source = std::io::BufReader::new(file);
                match rodio::Decoder::new(source) {
                    Ok(decoder) => {
                        // Prepend silence to avoid cut-in at playback start
                        let silence = rodio::source::SineWave::new(440.0)
                            .take_duration(Duration::from_millis(100))
                            .amplify(0.0);
                        sink.append(silence);
                        sink.append(decoder);
                        // Block this thread until sound completes to keep stream alive
                        sink.sleep_until_end();
                    }
                    Err(e) => eprintln!("Failed to decode sound {}: {}", path_buf.display(), e),
                }
            }
            Err(e) => eprintln!("Failed to open sound {}: {}", path_buf.display(), e),
        }
        // Dropping stream_handle here stops the mixer; after playback finished.
    });
}

#[derive(Clone, Copy, PartialEq)]
enum RecordingMode {
    Hold,
    Latch,
}

struct AudioManager {
    recorder: AudioRecorder,
    current_stream: Option<cpal::Stream>,
    mode: RecordingMode,
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
    start_time: Arc<Mutex<Option<Instant>>>,
}

impl Clone for AudioRecorder {
    fn clone(&self) -> Self {
        Self {
            recording: Arc::clone(&self.recording),
            sample_rate: self.sample_rate,
            channels: self.channels,
            tx: Arc::clone(&self.tx),
            result_rx: Arc::clone(&self.result_rx),
            start_time: Arc::clone(&self.start_time),
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
            start_time: Arc::new(Mutex::new(None)),
        }
    }

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

        *self.start_time.lock().unwrap() = Some(Instant::now());
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

    fn is_recording(&self) -> bool {
        *self.recording.lock().unwrap()
    }
}

impl AudioManager {
    fn new() -> Self {
        Self {
            recorder: AudioRecorder::new(),
            current_stream: None,
            mode: RecordingMode::Hold,
        }
    }

    fn start_recording(&mut self, mode: RecordingMode) -> Result<(), String> {
        if self.recorder.is_recording() {
            return Ok(());
        }

        self.mode = mode;
        self.recorder.configure_from_device()?;
        self.recorder.prepare_recording()?;

        /*
        Not only audio-out systems take time to wake up from sleep, but audio-in systems (like mic) take time to wake up as well.
        So the delay + small audio play is rather a beneficial side-effect gives the chance of audio-in to boot up too.

        From https://handy.computer 's author cjpais:
        Sometimes it doesn't pick up the first one or two words.

        Q. Sometimes it doesn't pick up the first one or two words.
        A. Maybe try the “always on microphone” setting and see if that helps,
           it can take the audio system some time to get all the necessary resources from the system.
           On macOS it’s about 100-200ms but I haven’t measured on other platforms.
        */
        play_sound("assets/on.mp3");

        let mode_str = match mode {
            RecordingMode::Hold => "HOLD",
            RecordingMode::Latch => "LATCH",
        };
        println!(
            "Started recording in {} mode ({}Hz, {} channels)",
            mode_str, self.recorder.sample_rate, self.recorder.channels
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
                                let clamped = avg.clamp(-1.0, 1.0);
                                mono.push((clamped * (i16::MAX as f32)) as i16);
                            }
                            mono
                        } else {
                            let mut mono = Vec::with_capacity(data.len());
                            for &s in data {
                                let clamped = s.clamp(-1.0, 1.0);
                                mono.push((clamped * (i16::MAX as f32)) as i16);
                            }
                            mono
                        };
                        if let Ok(guard) = tx_arc.lock()
                            && let Some(tx) = &*guard
                        {
                            let _ = tx.try_send(chunk);
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

        // Check recording duration before processing
        let duration = if let Some(start_time) = *self.recorder.start_time.lock().unwrap() {
            start_time.elapsed().as_secs_f64()
        } else {
            0.0
        };

        // Finalize recording
        if self.recorder.finalize_recording()? {
            // Skip processing if recording is too short
            if duration < 0.9 {
                println!(
                    "Recording too short ({:.2}s), skipping transcription",
                    duration
                );
                // Still need to consume the Opus data to clean up the worker
                if let Ok(mut guard) = self.recorder.tx.lock() {
                    guard.take();
                }
                let result_rx_arc = Arc::clone(&self.recorder.result_rx);
                std::thread::spawn(move || {
                    let mut guard = result_rx_arc.lock().unwrap();
                    if let Some(rx) = guard.take() {
                        let _ = rx.recv(); // Consume and discard
                    }
                });
                return Ok(());
            }

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
                    // Save the Ogg Opus file
                    if let Err(e) = save_ogg_file(&opus_data) {
                        eprintln!("Failed to save Ogg Opus file: {}", e);
                    }
                    // */

                    // Check for speech activity using VAD
                    match check_speech_activity(&opus_data) {
                        Ok(has_speech) => {
                            if has_speech {
                                play_sound("assets/off.mp3");
                                println!("Processing transcription...");
                                if let Err(e) = transcribe_audio_opus(opus_data) {
                                    eprintln!("Failed to transcribe audio: {}", e);
                                }
                            } else {
                                println!("No speech detected, skipping transcription");
                            }
                        }
                        Err(e) => {
                            eprintln!("VAD analysis failed: {}, proceeding with transcription", e);
                            play_sound("assets/off.mp3");
                            println!("Processing transcription...");
                            if let Err(e) = transcribe_audio_opus(opus_data) {
                                eprintln!("Failed to transcribe audio: {}", e);
                            }
                        }
                    }
                } else {
                    eprintln!("No Opus data produced");
                }
            });
        }

        Ok(())
    }

    fn switch_to_latch_mode(&mut self) -> Result<(), String> {
        if self.recorder.is_recording() && self.mode == RecordingMode::Hold {
            self.mode = RecordingMode::Latch;
            println!("Switched to LATCH mode - press AltGr/Right Cmd to stop");
            Ok(())
        } else {
            Ok(())
        }
    }
}

fn check_speech_activity(opus_data: &[u8]) -> Result<bool, String> {
    let vad_start = Instant::now();

    // Decode Opus to PCM for VAD analysis
    let mut decoder = match opus::Decoder::new(48000, opus::Channels::Mono) {
        Ok(d) => d,
        Err(e) => return Err(format!("Failed to create Opus decoder: {}", e)),
    };

    // Parse OGG container to extract Opus packets
    let mut ogg_reader = ogg::reading::PacketReader::new(std::io::Cursor::new(opus_data));
    let mut all_samples = Vec::new();
    let mut packet_count = 0;

    while let Some(packet) = ogg_reader
        .read_packet()
        .map_err(|e| format!("OGG read error: {}", e))?
    {
        if packet_count == 0 {
            // Skip first packet (Opus header)
            packet_count += 1;
            continue;
        }

        let mut pcm_buffer = vec![0i16; 960]; // 20ms at 48kHz
        match decoder.decode(&packet.data, &mut pcm_buffer, false) {
            Ok(samples) => {
                all_samples.extend_from_slice(&pcm_buffer[..samples]);
            }
            Err(e) => {
                eprintln!("Opus decode error: {}", e);
                continue;
            }
        }
        packet_count += 1;
    }

    if all_samples.is_empty() {
        return Ok(false);
    }

    // Convert i16 to f32 and downsample from 48kHz to 16kHz (3:1 ratio)
    let f32_samples: Vec<f32> = all_samples
        .iter()
        .step_by(3) // Simple downsampling by taking every 3rd sample
        .map(|&s| s as f32 / 32768.0)
        .collect();

    // Initialize VAD
    let mut vad = match VoiceActivityDetector::builder()
        .sample_rate(16000)
        .chunk_size(512usize)
        .build()
    {
        Ok(v) => v,
        Err(e) => return Err(format!("Failed to create VAD: {}", e)),
    };

    // Process audio in chunks suitable for VAD (512 samples for 48kHz)
    const CHUNK_SIZE: usize = 512;
    let mut speech_detected = false;

    for chunk in f32_samples.chunks(CHUNK_SIZE) {
        if chunk.len() == CHUNK_SIZE {
            let chunk_owned: Vec<f32> = chunk.to_vec();
            let is_speech = vad.predict(chunk_owned);
            if is_speech > 0.5 {
                // Threshold for speech detection
                speech_detected = true;
                break;
            }
        }
    }

    let vad_time = vad_start.elapsed();
    println!(
        "VAD analysis time: {:.3} ms, speech detected: {}",
        vad_time.as_secs_f64() * 1000.0,
        speech_detected
    );

    Ok(speech_detected)
}

fn _save_ogg_file(ogg_data: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let filename = format!("recording_{}.ogg", ts);
    let mut file = File::create(&filename)?;
    file.write_all(ogg_data)?;
    println!("Saved Ogg Opus audio to: {}", filename);
    Ok(filename)
}

fn build_groq_prompt(context_bias: &str) -> Option<String> {
    let mut terms = Vec::new();

    for term in context_bias.split(',').map(str::trim) {
        if term.is_empty() || terms.contains(&term) {
            continue;
        }
        terms.push(term);
    }

    if terms.is_empty() {
        return None;
    }

    let mut prompt = format!("Use these spellings if relevant: {}.", terms.join(", "));

    const MAX_PROMPT_CHARS: usize = 400;
    if prompt.len() > MAX_PROMPT_CHARS {
        prompt.truncate(MAX_PROMPT_CHARS);
    }

    Some(prompt)
}

fn transcribe_audio_opus(opus_data: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let api_key = env::var("GROQ_API_KEY").expect("GROQ_API_KEY environment variable must be set");
    let context_bias = env::var("CONTEXT_BIAS").ok();

    let client = reqwest::blocking::Client::new();

    let mut form = multipart::Form::new()
        .text("model", "whisper-large-v3-turbo")
        .text("language", "en")
        .text("response_format", "json")
        .text("temperature", "0")
        .part(
            "file",
            multipart::Part::bytes(opus_data)
                .file_name("audio.ogg")
                .mime_str("audio/ogg")?,
        );

    if let Some(prompt) = context_bias.as_deref().and_then(build_groq_prompt) {
        form = form.text("prompt", prompt);
    }

    println!("Sending Ogg Opus audio to Groq Whisper API...");
    let start_time = Instant::now();

    let response = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()?
        .error_for_status()?;

    let api_latency = start_time.elapsed();
    let transcription: TranscriptionResponse = response.json()?;

    println!("API Response Time: {:.2}ms", api_latency.as_millis());
    println!("Transcription: {}", transcription.text);

    // Store the transcription for later retyping
    let last_transcription_arc = LAST_TRANSCRIPTION.get_or_init(|| Arc::new(Mutex::new(None)));
    if let Ok(mut last_transcription) = last_transcription_arc.lock() {
        *last_transcription = Some(transcription.text.clone());
    }

    // Type the transcript into the active window
    type_transcript(&transcription.text);

    Ok(())
}

fn type_transcript(text: &str) {
    // Best effort: avoid panics; just log errors.
    // Enigo types into the currently focused window.

    // Normalize any model-provided surrounding whitespace, then add a trailing
    // space only when the transcript ends with sentence punctuation.
    let clean = text.trim();
    let to_type = if clean.ends_with(['.', '!', '?', ':', ';']) {
        format!("{} ", clean)
    } else {
        clean.to_string()
    };

    // Typing can take time; run in a detached thread so we don't block.
    let s = to_type;
    std::thread::spawn(move || {
        if let Err(e) = try_type(&s) {
            eprintln!("Typing error: {}", e);
        }
    });
}

fn try_type(text: &str) -> Result<(), String> {
    use enigo::{Enigo, Keyboard, Settings};
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("Enigo init error: {e}"))?;
    enigo
        .text(text)
        .map_err(|e| format!("Enigo text error: {e}"))?;
    Ok(())
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
    // Manager must stay on main thread (cpal stream is not Send/Sync)
    let mut audio_manager = AudioManager::new();

    println!("Groq Whisper Speech-to-Text");
    println!("Recording modes:");
    #[cfg(target_os = "macos")]
    {
        println!("  HOLD: Hold Right Cmd (⌘), release to transcribe");
        println!(
            "  LATCH: Press Space while in HOLD mode to switch to LATCH, then press Right Cmd to stop"
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        println!("  HOLD: Hold Right Alt (AltGr), release to transcribe");
        println!(
            "  LATCH: Press Space while in HOLD mode to switch to LATCH, then press AltGr to stop"
        );
    }
    println!("Other hotkeys:");
    #[cfg(target_os = "macos")]
    println!("  Right Cmd+' : Retype last transcription");
    #[cfg(not(target_os = "macos"))]
    println!("  AltGr+' : Retype last transcription");
    println!("Waiting for hotkey...");

    // Control channel from hotkey listener -> main thread
    enum ControlMsg {
        StopHold,
        SinglePress,
        SwitchToLatch,
        TypeLastTranscription,
        Quit,
    }
    let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel::<ControlMsg>();

    // Ctrl+C handler: request graceful shutdown
    {
        let tx = ctrl_tx.clone();
        ctrlc::set_handler(move || {
            let _ = tx.send(ControlMsg::Quit);
        })
        .expect("failed to set Ctrl+C handler");
    }

    // Hotkey control channel
    let tx1 = ctrl_tx.clone();

    // rdev listens on a blocking loop; run it in a thread
    thread::spawn(move || {
        use rdev::{EventType, Key};

        // Platform-specific modifier key
        #[cfg(target_os = "macos")]
        const MODIFIER_KEY: Key = Key::MetaRight;
        #[cfg(not(target_os = "macos"))]
        const MODIFIER_KEY: Key = Key::AltGr;

        // Track modifier key for combination detection
        let mut modifier_pressed = false;
        let mut quote_combo_active = false;

        let callback = move |event: rdev::Event| {
            match event.event_type {
                EventType::KeyPress(key) if key == MODIFIER_KEY => {
                    modifier_pressed = true;
                    let _ = tx1.send(ControlMsg::SinglePress);
                }
                EventType::KeyRelease(key) if key == MODIFIER_KEY => {
                    modifier_pressed = false;
                    let _ = tx1.send(ControlMsg::StopHold);
                }
                EventType::KeyPress(Key::Quote) => {
                    if modifier_pressed {
                        quote_combo_active = true;
                    }
                }
                EventType::KeyRelease(Key::Quote) => {
                    if quote_combo_active {
                        // Trigger when quote is released while combo was active
                        // Works regardless of whether modifier is still held
                        quote_combo_active = false;
                        let _ = tx1.send(ControlMsg::TypeLastTranscription);
                    }
                }
                EventType::KeyPress(Key::Space) => {
                    let _ = tx1.send(ControlMsg::SwitchToLatch);
                }
                _ => {}
            }
        };

        if let Err(e) = rdev::listen(callback) {
            eprintln!("Global hotkey listener error: {:?}", e);
        }
    });

    // Main thread: handle control messages and operate the audio manager
    loop {
        match ctrl_rx.recv() {
            Ok(ControlMsg::StopHold) => {
                if audio_manager.recorder.is_recording()
                    && audio_manager.mode == RecordingMode::Hold
                    && let Err(e) = audio_manager.stop_recording()
                {
                    eprintln!("Failed to stop recording: {}", e);
                }
            }
            Ok(ControlMsg::SinglePress) => {
                if audio_manager.recorder.is_recording()
                    && audio_manager.mode == RecordingMode::Latch
                {
                    if let Err(e) = audio_manager.stop_recording() {
                        eprintln!("Failed to stop recording: {}", e);
                    }
                } else if !audio_manager.recorder.is_recording()
                    && let Err(e) = audio_manager.start_recording(RecordingMode::Hold)
                {
                    eprintln!("Failed to start hold recording: {}", e);
                }
            }
            Ok(ControlMsg::SwitchToLatch) => {
                if let Err(e) = audio_manager.switch_to_latch_mode() {
                    eprintln!("Failed to switch to latch mode: {}", e);
                }
            }
            Ok(ControlMsg::TypeLastTranscription) => {
                let last_transcription_arc =
                    LAST_TRANSCRIPTION.get_or_init(|| Arc::new(Mutex::new(None)));
                if let Ok(last_transcription) = last_transcription_arc.lock() {
                    if let Some(ref text) = *last_transcription {
                        println!("Retyping last transcription: {}", text);
                        type_transcript(text);
                    } else {
                        println!("No previous transcription to retype");
                    }
                }
            }
            Ok(ControlMsg::Quit) => {
                // Gracefully stop if recording, then exit
                if audio_manager.recorder.is_recording()
                    && let Err(e) = audio_manager.stop_recording()
                {
                    eprintln!("Failed to stop recording: {}", e);
                }
                break;
            }
            Err(_) => break,
        }
    }

    Ok(())
}
