#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Hidden,
    Recording,
    RecordingLatch,
    Transcribing,
}
