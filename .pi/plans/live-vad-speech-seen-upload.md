# Plan: use live VAD speech flag for upload decision

## Goal

Avoid the post-recording Opus decode + 48 kHz -> 16 kHz VAD pass. During capture, keep using live VAD at **16 kHz PCM**. On stop, finalize the existing **48 kHz mono Ogg Opus @ 24 kbps** upload bytes and immediately send them to Mistral/Voxtral if live VAD observed speech.

User choices locked:

- Keep live VAD at 16 kHz PCM.
- Keep upload audio as current 48 kHz Opus @ 24 kbps.
- Remove final `check_speech_activity()` decode/VAD gate from the stop path.
- If live VAD cannot initialize, abort recording start with an error.
- Drop RMS-only fallback behavior.

## Current references

- `src/main.rs:221` `AudioRecorder` state.
- `src/main.rs:243` `AudioRecorder::new()`.
- `src/main.rs:315` `AudioManager::new()`.
- `src/main.rs:385` live VAD initialization currently falls back to RMS-only.
- `src/main.rs:397` live VAD resampling from input sample rate to 16 kHz.
- `src/main.rs:457-526` live VAD only gates visualization; no persistent final speech flag.
- `src/main.rs:621-668` finalization thread currently calls `check_speech_activity(&opus_data)` before upload.
- `src/main.rs:692` `check_speech_activity()` decodes Ogg Opus, downsamples to 16 kHz, and runs final VAD.
- `src/main.rs:1204` `OPUS_SAMPLE_RATE = 48_000`.
- `src/main.rs:1266` Opus worker resamples to 48 kHz before Opus.
- `src/main.rs:1276` Opus bitrate is 24 kbps.

## Design

Add a shared per-recording live VAD speech flag:

```rust
speech_seen: Arc<Mutex<bool>>
```

The audio callback sets it to `true` once live VAD enters speech. `stop_recording()` reads it after Opus finalization. If true, upload the Ogg Opus bytes immediately. If false, skip transcription.

This preserves the current upload format and network size while removing local duplicate work:

```text
before:
  capture PCM -> live VAD for UI
  capture PCM -> 48 kHz Opus @ 24 kbps
  stop -> decode Opus -> downsample to 16 kHz -> final VAD -> upload

after:
  capture PCM -> live VAD at 16 kHz; set speech_seen
  capture PCM -> 48 kHz Opus @ 24 kbps
  stop -> if speech_seen upload finalized Opus immediately
```

## Implementation patch spec

### 1. Extend `AudioRecorder`

In `src/main.rs`, update `struct AudioRecorder` near `start_time`:

```rust
struct AudioRecorder {
    recording: Arc<Mutex<bool>>,
    sample_rate: u32,
    channels: u16,
    tx: Arc<Mutex<Option<flume::Sender<Vec<i16>>>>>,
    result_rx: Arc<Mutex<Option<flume::Receiver<Vec<u8>>>>>,
    start_time: Arc<Mutex<Option<Instant>>>,
    speech_seen: Arc<Mutex<bool>>,
}
```

Update `impl Clone for AudioRecorder` to clone `speech_seen`:

```rust
speech_seen: Arc::clone(&self.speech_seen),
```

Update `AudioRecorder::new()`:

```rust
speech_seen: Arc::new(Mutex::new(false)),
```

### 2. Reset speech flag at recording start

In `AudioRecorder::prepare_recording()`, before `*recording = true;`, reset:

```rust
*self.speech_seen.lock().unwrap() = false;
```

Suggested placement: after `*self.start_time.lock().unwrap() = Some(Instant::now());`.

### 3. Make live VAD mandatory

Replace current live VAD initialization in `AudioManager::start_recording()`:

```rust
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
```

with:

```rust
let mut live_vad = VoiceActivityDetector::builder()
    .sample_rate(16_000)
    .chunk_size(LIVE_VAD_CHUNK_SIZE)
    .build()
    .map_err(|e| format!("Live VAD init failed: {e}"))?;
```

This implements “abort recording start with an error if live VAD cannot initialize”.

### 4. Capture `speech_seen` in the audio callback

Near existing callback captures:

```rust
let tx_arc = Arc::clone(&self.recorder.tx);
let speech_viz = Arc::clone(&self.speech_viz);
```

add:

```rust
let speech_seen = Arc::clone(&self.recorder.speech_seen);
```

The `move` callback will then own a clone of the shared flag.

### 5. Remove RMS-only fallback branches from callback logic

Because live VAD is now mandatory, simplify:

- Remove `let live_vad_enabled = live_vad.is_some();`.
- Replace `if let Some(vad) = live_vad.as_mut() { ... }` with direct use of `live_vad`.
- Remove the `else { smoothed_level }` branch that handled no VAD.

Concretely, transform this shape:

```rust
let live_vad_enabled = live_vad.is_some();
let mut vad_gate = 1.0f32;

if let Some(vad) = live_vad.as_mut() {
    ...
}

let gated_target = if live_vad_enabled {
    ...
} else {
    smoothed_level
};
```

into:

```rust
let mut vad_gate = 1.0f32;

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
    let frame: Vec<f32> = vad_buffer.drain(..LIVE_VAD_CHUNK_SIZE).collect();
    let raw = live_vad.predict(frame).clamp(0.0, 1.0);
    speech_conf = ((1.0 - VAD_SMOOTH_ALPHA) * speech_conf) + (VAD_SMOOTH_ALPHA * raw);
}

if in_speech {
    if speech_conf < VAD_EXIT_THRESHOLD {
        in_speech = false;
    }
} else if speech_conf > VAD_ENTER_THRESHOLD {
    in_speech = true;
    if let Ok(mut seen) = speech_seen.lock() {
        *seen = true;
    }
}

vad_gate = if in_speech {
    1.0
} else {
    (speech_conf / VAD_ENTER_THRESHOLD).clamp(0.0, 1.0) * 0.35
};

let gated_target = {
    if in_speech {
        noise_floor = (noise_floor * 0.996) + (rms * 0.004);
    } else {
        noise_floor = (noise_floor * 0.94) + (rms * 0.06);
    }
    noise_floor = noise_floor.clamp(0.0008, 0.12);

    let noise_ref = (noise_floor * 1.14).max(0.0012);
    let snr_gate = ((rms - noise_ref) / (noise_ref * 2.8)).clamp(0.0, 1.0);

    vad_gate_smoothed += GATE_SMOOTH_ALPHA * (vad_gate - vad_gate_smoothed);
    snr_gate_smoothed += GATE_SMOOTH_ALPHA * (snr_gate - snr_gate_smoothed);

    let blended_gate = if in_speech {
        ((0.72 * vad_gate_smoothed) + (0.28 * snr_gate_smoothed)).clamp(0.52, 1.0)
    } else {
        ((0.62 * vad_gate_smoothed) + (0.38 * snr_gate_smoothed)).clamp(0.0, 0.55)
    };

    (smoothed_level * blended_gate).clamp(0.0, 1.0)
};
```

Note: `std::sync::Mutex::lock()` in the real-time-ish audio callback is usually best avoided, but this lock occurs only at speech-entry transition, not per sample/chunk once speech has been observed. If desired during implementation, reduce locking further by checking a local `speech_marked` boolean captured in the closure:

```rust
let mut speech_marked = false;
...
if !speech_marked && speech_conf > VAD_ENTER_THRESHOLD {
    speech_marked = true;
    *speech_seen.lock().unwrap() = true;
}
```

This is recommended.

### 6. Use `speech_seen` in `stop_recording()` finalization thread

In `AudioManager::stop_recording()`, before spawning the finalization thread, add:

```rust
let speech_seen = Arc::clone(&self.recorder.speech_seen);
```

Move it into the thread.

Replace the current final VAD block:

```rust
// Check for speech activity using VAD
match check_speech_activity(&opus_data) {
    Ok(has_speech) => { ... }
    Err(e) => { ... proceed with transcription ... }
}
```

with:

```rust
let has_speech = *speech_seen.lock().unwrap();
if has_speech {
    sender_clone.send(AppEvent::Overlay(OverlayState::Transcribing));
    play_sound(OFF_SOUND_PATH);
    println!("Processing transcription...");
    if let Err(e) = transcribe_audio_opus(opus_data, sender_clone.clone()) {
        eprintln!("Failed to transcribe audio: {}", e);
    }
    sender_clone.send(AppEvent::Overlay(OverlayState::Hidden));
} else {
    println!("No speech detected by live VAD, skipping transcription");
    sender_clone.send(AppEvent::Overlay(OverlayState::Hidden));
}
```

This removes the second decode/resample/VAD stage from the critical stop-to-upload path.

### 7. Remove dead final VAD function

After the stop path no longer references it, delete:

```rust
fn check_speech_activity(opus_data: &[u8]) -> Result<bool, String> { ... }
```

This also removes dependence on `opus::Decoder` usage for final VAD, though the `opus` crate remains needed for encoding.

### 8. Update comments/logging

Suggested changes:

- Replace “During VAD analysis keep overlay hidden; show spinner only if transcription starts.” with something like:

```rust
// Keep overlay hidden while finalizing audio; show spinner only if transcription starts.
```

- Add a concise log when speech flag is set if useful, but avoid logging in the callback repeatedly. Prefer no callback log.

## Verification

Run:

```bash
cargo fmt
cargo check
cargo clippy
```

Manual runtime checks:

1. Start and stop without speaking.
   - Expected: no API call, log says live VAD detected no speech.
2. Speak briefly and stop.
   - Expected: after audio finalization, overlay switches to transcribing immediately; no `VAD analysis time` log appears.
3. Temporarily force VAD builder failure if practical, or inject an invalid VAD config in a throwaway patch.
   - Expected: recording start returns `Live VAD init failed: ...`, no RMS-only fallback.
4. Confirm upload format unchanged by logs/code:
   - `OPUS_SAMPLE_RATE` remains `48_000`.
   - bitrate remains `opus::Bitrate::Bits(24000)`.
   - multipart upload remains `audio.ogg` / `audio/ogg`.

## Expected impact

- Faster stop-to-upload path by removing full Ogg Opus decode and final 16 kHz VAD analysis.
- No intentional network size change.
- No intentional upload format change.
- Breaking behavior change: if live VAD fails to initialize, recording start fails instead of falling back to RMS-only visualization and final VAD.
