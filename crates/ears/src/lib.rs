//! # Ears — Audio capture and analysis
//!
//! This crate handles the real-time audio pipeline:
//! - Microphone capture via cpal
//! - MIDI input via midir
//! - Pitch detection via Aubio/PESTO
//! - Onset detection
//! - Instrument profile loading

pub mod capture;
pub mod output;
pub mod pitch;
pub mod profile;

/// Audio event emitted by the Ears layer to the Brain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioEvent {
    /// Detected pitch in Hz (None if silence or unpitched)
    pub pitch_hz: Option<f64>,
    /// Confidence of the pitch detection (0.0 to 1.0)
    pub confidence: f64,
    /// RMS amplitude
    pub amplitude: f64,
    /// Timestamp in seconds from session start
    pub timestamp_secs: f64,
    /// Whether an onset (note attack) was detected
    pub is_onset: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_event_serialization_roundtrip() {
        let event = AudioEvent {
            pitch_hz: Some(440.0),
            confidence: 0.95,
            amplitude: 0.8,
            timestamp_secs: 1.234,
            is_onset: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AudioEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pitch_hz, Some(440.0));
        assert!(parsed.is_onset);
    }
}
