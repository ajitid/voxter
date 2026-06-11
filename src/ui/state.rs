#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Hidden,
    Recording,
    #[cfg(target_os = "macos")]
    RecordingLatch,
    Transcribing,
}
