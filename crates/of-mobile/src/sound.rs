//! Replaces the PC version's `sound.rs` (same API as far as its modules use
//! it: `Sound::start`, `Sound::play`). Stage 0 is silent; the alert sound
//! goes through the notification channel later (plan, stage 3).

#[derive(Clone)]
pub struct Sound;

impl Sound {
    pub fn start() -> Sound {
        Sound
    }

    /// `volume` 0..1; `file` empty for the built-in sound.
    pub fn play(&self, volume: f32, file: &str) {
        log::debug!("alert sound requested (volume {volume}, file {file:?}); not played yet");
    }
}
