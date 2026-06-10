mod ui;

use reqwest::blocking::multipart;
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Instant;
use ui::overlay::{OverlayController, OverlayState};
#[cfg(target_os = "macos")]
use ui::tray::{StatusTray, build_status_tray};
use voice_activity_detector::VoiceActivityDetector;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
use winit::window::WindowId;

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

#[derive(Clone)]
enum ControlMsg {
    StopHold,
    SinglePress,
    SwitchToLatch,
    Quit,
}

#[derive(Clone)]
enum AppEvent {
    Control(ControlMsg),
    Overlay(OverlayState),
    #[cfg(target_os = "macos")]
    TrayMenu(tray_icon::menu::MenuEvent),
    TranscriptUpdated,
}

pub(crate) struct SpeechVizState {
    rms_norm_bits: AtomicU32,
    active: AtomicBool,
}

impl SpeechVizState {
    fn new() -> Self {
        Self {
            rms_norm_bits: AtomicU32::new(0.0f32.to_bits()),
            active: AtomicBool::new(false),
        }
    }

    fn set_level(&self, value: f32) {
        let clamped = value.clamp(0.0, 1.0);
        self.rms_norm_bits
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn level(&self) -> f32 {
        f32::from_bits(self.rms_norm_bits.load(Ordering::Relaxed))
    }

    fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    fn deactivate(&self) {
        self.set_active(false);
    }

    fn reset(&self) {
        self.set_level(0.0);
        self.set_active(false);
    }
}

struct AudioManager {
    recorder: AudioRecorder,
    current_stream: Option<cpal::Stream>,
    mode: RecordingMode,
    speech_viz: Arc<SpeechVizState>,
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
                eprintln!("Audio encoding error: {}", e);
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
            speech_viz: Arc::new(SpeechVizState::new()),
        }
    }

    fn start_recording(&mut self, mode: RecordingMode) -> Result<(), String> {
        if self.recorder.is_recording() {
            return Ok(());
        }

        self.mode = mode;
        self.recorder.configure_from_device()?;
        self.recorder.prepare_recording()?;
        self.speech_viz.set_active(true);
        self.speech_viz.set_level(0.0);

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
        let speech_viz = Arc::clone(&self.speech_viz);

        const RMS_GAIN: f32 = 4.8;
        const RMS_GAMMA: f32 = 0.52;
        const ATTACK_ALPHA: f32 = 0.40;
        const RELEASE_ALPHA: f32 = 0.16;

        const VIS_ATTACK_ALPHA: f32 = 0.50;
        const VIS_RELEASE_ALPHA: f32 = 0.22;
        const GATE_SMOOTH_ALPHA: f32 = 0.14;
        const VIS_NOISE_DEADZONE: f32 = 0.055;

        const LIVE_VAD_SAMPLE_RATE: f32 = 16_000.0;
        const LIVE_VAD_CHUNK_SIZE: usize = 512;
        const VAD_SMOOTH_ALPHA: f32 = 0.24;
        const VAD_ENTER_THRESHOLD: f32 = 0.42;
        const VAD_EXIT_THRESHOLD: f32 = 0.30;

        let mut live_vad = match VoiceActivityDetector::builder()
            .sample_rate(16_000)
            .chunk_size(LIVE_VAD_CHUNK_SIZE)
            .build()
        {
            Ok(vad) => Some(vad),
            Err(e) => {
                eprintln!("Live VAD init failed (falling back to RMS-only): {e}");
                None
            }
        };

        let input_sample_rate = config.sample_rate().0 as f32;
        let vad_resample_step = (input_sample_rate / LIVE_VAD_SAMPLE_RATE).max(0.01);

        let channels_cfg = config.channels() as usize;
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let mut smoothed_level = 0.0f32;
                let mut final_visual_level = 0.0f32;
                let mut vad_gate_smoothed = 1.0f32;
                let mut snr_gate_smoothed = 1.0f32;
                let mut vad_buffer = Vec::<f32>::with_capacity(LIVE_VAD_CHUNK_SIZE * 3);
                let mut resample_phase = 0.0f32;
                let mut speech_conf = 0.0f32;
                let mut in_speech = false;
                let mut noise_floor = 0.0035f32;
                device
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

                            if !chunk.is_empty() {
                                let sum_sq: f32 = chunk
                                    .iter()
                                    .map(|&s| {
                                        let x = s as f32 / 32768.0;
                                        x * x
                                    })
                                    .sum();
                                let rms = (sum_sq / chunk.len() as f32).sqrt();
                                let normalized = (rms * RMS_GAIN).powf(RMS_GAMMA).clamp(0.0, 1.0);
                                let alpha = if normalized > smoothed_level {
                                    ATTACK_ALPHA
                                } else {
                                    RELEASE_ALPHA
                                };
                                smoothed_level += alpha * (normalized - smoothed_level);

                                let live_vad_enabled = live_vad.is_some();
                                let mut vad_gate = 1.0f32;

                                if let Some(vad) = live_vad.as_mut() {
                                    let mut idx = resample_phase;
                                    let chunk_len_f = chunk.len() as f32;
                                    while idx < chunk_len_f {
                                        let sample_idx = idx as usize;
                                        if sample_idx >= chunk.len() {
                                            break;
                                        }
                                        vad_buffer.push(chunk[sample_idx] as f32 / 32768.0);
                                        idx += vad_resample_step;
                                    }
                                    resample_phase = idx - chunk_len_f;

                                    while vad_buffer.len() >= LIVE_VAD_CHUNK_SIZE {
                                        let frame: Vec<f32> =
                                            vad_buffer.drain(..LIVE_VAD_CHUNK_SIZE).collect();
                                        let raw = vad.predict(frame).clamp(0.0, 1.0);
                                        speech_conf = ((1.0 - VAD_SMOOTH_ALPHA) * speech_conf)
                                            + (VAD_SMOOTH_ALPHA * raw);
                                    }

                                    if in_speech {
                                        if speech_conf < VAD_EXIT_THRESHOLD {
                                            in_speech = false;
                                        }
                                    } else if speech_conf > VAD_ENTER_THRESHOLD {
                                        in_speech = true;
                                    }

                                    vad_gate = if in_speech {
                                        1.0
                                    } else {
                                        (speech_conf / VAD_ENTER_THRESHOLD).clamp(0.0, 1.0) * 0.35
                                    };
                                }

                                let gated_target = if live_vad_enabled {
                                    if in_speech {
                                        noise_floor = (noise_floor * 0.996) + (rms * 0.004);
                                    } else {
                                        noise_floor = (noise_floor * 0.94) + (rms * 0.06);
                                    }
                                    noise_floor = noise_floor.clamp(0.0008, 0.12);

                                    let noise_ref = (noise_floor * 1.14).max(0.0012);
                                    let snr_gate =
                                        ((rms - noise_ref) / (noise_ref * 2.8)).clamp(0.0, 1.0);

                                    vad_gate_smoothed +=
                                        GATE_SMOOTH_ALPHA * (vad_gate - vad_gate_smoothed);
                                    snr_gate_smoothed +=
                                        GATE_SMOOTH_ALPHA * (snr_gate - snr_gate_smoothed);

                                    let blended_gate = if in_speech {
                                        ((0.72 * vad_gate_smoothed) + (0.28 * snr_gate_smoothed))
                                            .clamp(0.52, 1.0)
                                    } else {
                                        ((0.62 * vad_gate_smoothed) + (0.38 * snr_gate_smoothed))
                                            .clamp(0.0, 0.55)
                                    };

                                    (smoothed_level * blended_gate).clamp(0.0, 1.0)
                                } else {
                                    smoothed_level
                                };

                                let gated_target = if in_speech {
                                    gated_target
                                } else if gated_target <= VIS_NOISE_DEADZONE {
                                    0.0
                                } else {
                                    (((gated_target - VIS_NOISE_DEADZONE)
                                        / (1.0 - VIS_NOISE_DEADZONE))
                                        * 0.85)
                                        .clamp(0.0, 1.0)
                                };

                                let vis_alpha = if gated_target > final_visual_level {
                                    VIS_ATTACK_ALPHA
                                } else {
                                    VIS_RELEASE_ALPHA
                                };
                                final_visual_level +=
                                    vis_alpha * (gated_target - final_visual_level);
                                speech_viz.set_level(final_visual_level.clamp(0.0, 1.0));
                            }

                            if let Ok(guard) = tx_arc.lock()
                                && let Some(tx) = &*guard
                            {
                                let _ = tx.try_send(chunk);
                            }
                        },
                        |err| eprintln!("Audio stream error: {}", err),
                        None,
                    )
                    .map_err(|e| format!("Failed to build input stream: {}", e))?
            }
            _ => return Err("Unsupported sample format".into()),
        };

        if let Err(e) = stream.play() {
            self.speech_viz.reset();
            return Err(format!("Failed to start stream: {}", e));
        }
        self.current_stream = Some(stream);

        Ok(())
    }

    fn stop_recording(&mut self, proxy: EventLoopProxy<AppEvent>) -> Result<(), String> {
        if !self.recorder.is_recording() {
            self.speech_viz.deactivate();
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
                let _ = proxy.send_event(AppEvent::Overlay(OverlayState::Hidden));
                return Ok(());
            }

            // During VAD analysis keep overlay hidden; show spinner only if transcription starts.
            let _ = proxy.send_event(AppEvent::Overlay(OverlayState::Hidden));

            // Close the sender to signal worker end-of-stream
            if let Ok(mut guard) = self.recorder.tx.lock() {
                guard.take();
            }
            let result_rx_arc = Arc::clone(&self.recorder.result_rx);
            let proxy_clone = proxy.clone();
            std::thread::spawn(move || {
                println!("Finalizing audio stream...");
                let start = Instant::now();
                let opus_data = {
                    let mut guard = result_rx_arc.lock().unwrap();
                    match guard.take().unwrap().recv() {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            eprintln!("Failed to receive audio data: {}", e);
                            Vec::new()
                        }
                    }
                };
                let dt = start.elapsed();
                println!("Audio finalize time: {:.3} ms", dt.as_secs_f64() * 1000.0);
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
                                let _ = proxy_clone
                                    .send_event(AppEvent::Overlay(OverlayState::Transcribing));
                                play_sound("assets/off.mp3");
                                println!("Processing transcription...");
                                if let Err(e) =
                                    transcribe_audio_opus(opus_data, proxy_clone.clone())
                                {
                                    eprintln!("Failed to transcribe audio: {}", e);
                                }
                                let _ =
                                    proxy_clone.send_event(AppEvent::Overlay(OverlayState::Hidden));
                            } else {
                                println!("No speech detected, skipping transcription");
                                let _ =
                                    proxy_clone.send_event(AppEvent::Overlay(OverlayState::Hidden));
                            }
                        }
                        Err(e) => {
                            eprintln!("VAD analysis failed: {}, proceeding with transcription", e);
                            let _ = proxy_clone
                                .send_event(AppEvent::Overlay(OverlayState::Transcribing));
                            play_sound("assets/off.mp3");
                            println!("Processing transcription...");
                            if let Err(e) = transcribe_audio_opus(opus_data, proxy_clone.clone()) {
                                eprintln!("Failed to transcribe audio: {}", e);
                            }
                            let _ = proxy_clone.send_event(AppEvent::Overlay(OverlayState::Hidden));
                        }
                    }
                } else {
                    eprintln!("No audio data produced");
                    let _ = proxy_clone.send_event(AppEvent::Overlay(OverlayState::Hidden));
                }
            });
        }

        Ok(())
    }

    fn switch_to_latch_mode(&mut self) -> Result<bool, String> {
        if self.recorder.is_recording() && self.mode == RecordingMode::Hold {
            self.mode = RecordingMode::Latch;
            println!("Switched to LATCH mode - press Right Option/Alt to stop");
            Ok(true)
        } else {
            Ok(false)
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
                eprintln!("Audio decode error: {}", e);
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

fn parse_context_bias(context_bias: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();

    for term in context_bias.split(',').map(str::trim) {
        if term.is_empty() {
            continue;
        }
        // Mistral requires terms without spaces/commas - replace spaces with underscores
        let normalized = term.replace(' ', "_");
        if !terms.contains(&normalized) {
            terms.push(normalized);
        }
    }

    terms
}

fn transcribe_audio_opus(
    opus_data: Vec<u8>,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let api_key =
        env::var("MISTRAL_API_KEY").expect("MISTRAL_API_KEY environment variable must be set");
    let context_bias = env::var("CONTEXT_BIAS").ok();

    let client = reqwest::blocking::Client::new();

    let mut form = multipart::Form::new()
        .text("model", "voxtral-mini-latest")
        .text("language", "en")
        .part(
            "file",
            multipart::Part::bytes(opus_data)
                .file_name("audio.ogg")
                .mime_str("audio/ogg")?,
        );

    // Add context bias terms as array (each term gets its own form field)
    if let Some(bias) = context_bias.as_deref() {
        for term in parse_context_bias(bias) {
            form = form.text("context_bias", term);
        }
    }

    println!("Sending OGG audio to Mistral Voxtral API...");
    let start_time = Instant::now();

    let response = client
        .post("https://api.mistral.ai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()?;

    let api_latency = start_time.elapsed();

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .unwrap_or_else(|_| "Unable to read response body".to_string());
        return Err(format!("API error ({}): {}", status, body).into());
    }

    let transcription: TranscriptionResponse = response.json()?;

    let clean_text = transcription.text.trim().to_string();

    println!("API Response Time: {:.2}ms", api_latency.as_millis());
    println!("Transcription: {}", clean_text);

    let last_transcription_arc = LAST_TRANSCRIPTION.get_or_init(|| Arc::new(Mutex::new(None)));
    if let Ok(mut last_transcription) = last_transcription_arc.lock() {
        *last_transcription = Some(clean_text.clone());
    }
    let _ = proxy.send_event(AppEvent::TranscriptUpdated);

    // Type the transcript into the active window
    type_transcript(&clean_text);

    Ok(())
}

#[cfg(target_os = "macos")]
fn last_transcription_text() -> Option<String> {
    let last_transcription_arc = LAST_TRANSCRIPTION.get_or_init(|| Arc::new(Mutex::new(None)));
    last_transcription_arc
        .lock()
        .ok()
        .and_then(|value| value.clone())
}

#[cfg(target_os = "macos")]
fn type_last_transcript() -> Result<bool, String> {
    let Some(text) = last_transcription_text() else {
        return Ok(false);
    };

    type_transcript(&text);
    Ok(true)
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

struct App {
    audio_manager: AudioManager,
    overlay: Option<OverlayController>,
    overlay_window_id: Option<WindowId>,
    #[cfg(target_os = "macos")]
    tray: Option<StatusTray>,
    proxy: EventLoopProxy<AppEvent>,
}

impl App {
    fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            audio_manager: AudioManager::new(),
            overlay: None,
            overlay_window_id: None,
            #[cfg(target_os = "macos")]
            tray: None,
            proxy,
        }
    }

    fn update_loop_mode(&self, event_loop: &ActiveEventLoop) {
        let visible = self
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.is_visible());
        event_loop.set_control_flow(if visible {
            ControlFlow::Poll
        } else {
            ControlFlow::Wait
        });
    }

    #[cfg(target_os = "macos")]
    fn refresh_tray_menu_state(&self) {
        if let Some(tray) = self.tray.as_ref() {
            let enabled = last_transcription_text().is_some();
            tray.type_item.set_enabled(enabled);
        }
    }

    #[cfg(target_os = "macos")]
    fn handle_tray_menu_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: tray_icon::menu::MenuEvent,
    ) {
        let Some(tray) = self.tray.as_ref() else {
            return;
        };

        if event.id == tray.type_item.id() {
            match type_last_transcript() {
                Ok(true) => println!("Typed last transcript"),
                Ok(false) => println!("No last transcript available to type"),
                Err(e) => eprintln!("Failed to type transcript: {e}"),
            }
            self.refresh_tray_menu_state();
        } else if event.id == tray.quit_item.id() {
            self.handle_control(event_loop, ControlMsg::Quit);
        }
    }

    fn handle_control(&mut self, event_loop: &ActiveEventLoop, msg: ControlMsg) {
        match msg {
            ControlMsg::StopHold => {
                if self.audio_manager.recorder.is_recording()
                    && self.audio_manager.mode == RecordingMode::Hold
                    && let Err(e) = self.audio_manager.stop_recording(self.proxy.clone())
                {
                    eprintln!("Failed to stop recording: {}", e);
                }
            }
            ControlMsg::SinglePress => {
                if self.audio_manager.recorder.is_recording()
                    && self.audio_manager.mode == RecordingMode::Latch
                {
                    if let Err(e) = self.audio_manager.stop_recording(self.proxy.clone()) {
                        eprintln!("Failed to stop recording: {}", e);
                    }
                } else if !self.audio_manager.recorder.is_recording()
                    && let Err(e) = self.audio_manager.start_recording(RecordingMode::Hold)
                {
                    eprintln!("Failed to start hold recording: {}", e);
                } else {
                    let _ = self
                        .proxy
                        .send_event(AppEvent::Overlay(OverlayState::Recording));
                }
            }
            ControlMsg::SwitchToLatch => match self.audio_manager.switch_to_latch_mode() {
                Ok(true) => {
                    let _ = self
                        .proxy
                        .send_event(AppEvent::Overlay(OverlayState::RecordingLatch));
                }
                Ok(false) => {}
                Err(e) => eprintln!("Failed to switch to latch mode: {}", e),
            },
            ControlMsg::Quit => {
                if self.audio_manager.recorder.is_recording()
                    && let Err(e) = self.audio_manager.stop_recording(self.proxy.clone())
                {
                    eprintln!("Failed to stop recording: {}", e);
                }
                let _ = self
                    .proxy
                    .send_event(AppEvent::Overlay(OverlayState::Hidden));
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.overlay.is_none() {
            match OverlayController::new(event_loop, Arc::clone(&self.audio_manager.speech_viz)) {
                Ok(overlay) => {
                    self.overlay_window_id = Some(overlay.window_id());
                    self.overlay = Some(overlay);
                }
                Err(e) => eprintln!("Overlay initialization failed: {}", e),
            }
        }

        #[cfg(target_os = "macos")]
        {
            if self.tray.is_none() {
                match build_status_tray() {
                    Ok(tray) => self.tray = Some(tray),
                    Err(e) => eprintln!("Tray initialization failed: {e}"),
                }
            }
            self.refresh_tray_menu_state();
        }
        self.update_loop_mode(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Control(msg) => self.handle_control(event_loop, msg),
            AppEvent::Overlay(state) => {
                if state == OverlayState::Hidden {
                    self.audio_manager.speech_viz.reset();
                }
                if let Some(overlay) = self.overlay.as_mut()
                    && let Err(e) = overlay.update_state(event_loop, state)
                {
                    eprintln!("Overlay update failed: {e}");
                }
                self.update_loop_mode(event_loop);
            }
            #[cfg(target_os = "macos")]
            AppEvent::TrayMenu(event) => self.handle_tray_menu_event(event_loop, event),
            AppEvent::TranscriptUpdated => {
                #[cfg(target_os = "macos")]
                self.refresh_tray_menu_state();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.overlay_window_id {
            return;
        }

        if let Some(overlay) = self.overlay.as_mut() {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    let scale_factor = overlay.window().scale_factor();
                    overlay.handle_resize(size, scale_factor);
                }
                WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                    let size = overlay.window().inner_size();
                    overlay.handle_resize(size, scale_factor);
                }
                WindowEvent::RedrawRequested => {
                    if let Err(e) = overlay.redraw() {
                        eprintln!("Overlay redraw failed: {}", e);
                    }
                }
                _ => {}
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(overlay) = self.overlay.as_ref()
            && overlay.is_visible()
        {
            overlay.request_redraw();
        }
    }
}

#[cfg(target_os = "macos")]
fn spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>) {
    thread::spawn(move || {
        use rdev::set_is_main_thread;
        use rdev::{EventType, Key};

        set_is_main_thread(false);

        let callback = move |event: rdev::Event| match event.event_type {
            EventType::KeyPress(Key::AltGr) => {
                let _ = proxy.send_event(AppEvent::Control(ControlMsg::SinglePress));
            }
            EventType::KeyRelease(Key::AltGr) => {
                let _ = proxy.send_event(AppEvent::Control(ControlMsg::StopHold));
            }
            EventType::KeyPress(Key::Space) => {
                let _ = proxy.send_event(AppEvent::Control(ControlMsg::SwitchToLatch));
            }
            _ => {}
        };

        if let Err(e) = rdev::listen(callback) {
            eprintln!("Global hotkey listener error: {:?}", e);
        }
    });
}

#[cfg(target_os = "linux")]
#[derive(Deserialize)]
struct HelperHotkeyEvent {
    event: String,
}

#[cfg(target_os = "linux")]
fn spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>) {
    const PKEXEC_PATH: &str = "/usr/bin/pkexec";
    const HELPER_PATH: &str = "/usr/local/libexec/voxter-hotkey-helper";

    thread::spawn(move || {
        use std::io::BufRead;
        use std::process::{Command, Stdio};

        if !std::path::Path::new(HELPER_PATH).exists() {
            eprintln!(
                "Linux hotkey helper not found at {HELPER_PATH}. Install it with: scripts/install-linux-helper.sh"
            );
            return;
        }

        let mut child = match Command::new(PKEXEC_PATH)
            .arg(HELPER_PATH)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!(
                    "pkexec not found at {PKEXEC_PATH}; install polkit/pkexec to use Linux hotkeys"
                );
                return;
            }
            Err(error) => {
                eprintln!("Failed to launch Linux hotkey helper through pkexec: {error}");
                return;
            }
        };

        let Some(stdout) = child.stdout.take() else {
            eprintln!("Failed to capture Linux hotkey helper stdout");
            return;
        };

        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    eprintln!("Failed to read Linux hotkey helper event: {error}");
                    break;
                }
            };

            let event = match serde_json::from_str::<HelperHotkeyEvent>(&line) {
                Ok(event) => event,
                Err(error) => {
                    eprintln!("Ignoring malformed Linux hotkey helper event {line:?}: {error}");
                    continue;
                }
            };

            let control = match event.event.as_str() {
                "right_alt_press" => ControlMsg::SinglePress,
                "right_alt_release" => ControlMsg::StopHold,
                "space_press" => ControlMsg::SwitchToLatch,
                other => {
                    eprintln!("Ignoring unknown Linux hotkey helper event: {other}");
                    continue;
                }
            };

            let _ = proxy.send_event(AppEvent::Control(control));
        }

        match child.wait() {
            Ok(status) => match status.code() {
                Some(0) => eprintln!("Linux hotkey helper exited"),
                Some(126) => {
                    eprintln!("Linux hotkey helper authorization was cancelled by the user")
                }
                Some(127) => eprintln!(
                    "Linux hotkey helper authorization failed or helper is unavailable. Reinstall with: scripts/install-linux-helper.sh"
                ),
                Some(code) => eprintln!("Linux hotkey helper exited with status code {code}"),
                None => eprintln!("Linux hotkey helper was terminated by signal"),
            },
            Err(error) => eprintln!("Failed to wait for Linux hotkey helper: {error}"),
        }
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Mistral Voxtral Speech-to-Text");
    println!("Recording modes:");
    #[cfg(target_os = "macos")]
    {
        println!("  HOLD: Hold Right Option (⌥), release to transcribe");
        println!(
            "  LATCH: Press Space while in HOLD mode to switch to LATCH, then press Right Option again to stop"
        );
    }
    #[cfg(target_os = "linux")]
    {
        println!("  HOLD: Hold Right Alt / AltGr, release to transcribe");
        println!(
            "  LATCH: Press Space while in HOLD mode to switch to LATCH, then press Right Alt / AltGr again to stop"
        );
    }
    #[cfg(target_os = "macos")]
    {
        println!("Menu bar:");
        println!("  Use the microphone icon to type the last transcript or quit");
    }
    println!("Waiting for hotkey...");

    let mut event_loop_builder = EventLoop::<AppEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        event_loop_builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false);
    }
    let event_loop = event_loop_builder.build()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let proxy = event_loop.create_proxy();

    {
        let quit_proxy = proxy.clone();
        ctrlc::set_handler(move || {
            let _ = quit_proxy.send_event(AppEvent::Control(ControlMsg::Quit));
        })
        .expect("failed to set Ctrl+C handler");
    }

    #[cfg(target_os = "macos")]
    {
        let menu_proxy = proxy.clone();
        tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| {
            let _ = menu_proxy.send_event(AppEvent::TrayMenu(event));
        }));
    }

    spawn_hotkey_listener(proxy.clone());

    let mut app = App::new(proxy);
    event_loop.run_app(&mut app)?;
    Ok(())
}
