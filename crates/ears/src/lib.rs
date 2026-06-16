//! # Ears — Audio capture and analysis
//!
//! This crate handles the real-time audio pipeline:
//! - Microphone capture via cpal
//! - MIDI input via midir
//! - Pitch detection via Aubio/PESTO
//! - Onset detection
//! - Instrument profile loading
//! - Audio output: practice-aid sample generators ([`output`]) and a cpal
//!   output engine fed by a lock-free ring buffer ([`output_engine`])

pub mod capture;
pub mod onset;
pub mod output;
pub mod output_engine;
pub mod pitch;
pub mod profile;
pub mod recorder;
pub mod retention;

/// Computed note information from a detected pitch.
/// Present when pitch_hz is Some and the frequency maps to a valid MIDI note.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NoteInfo {
    /// MIDI note number (0–127, A4 = 69)
    pub midi_note: u8,
    /// Name of the note (e.g. "A", "C#")
    pub note_name: String,
    /// Octave number (A4 is octave 4)
    pub octave: i8,
    /// Cents deviation from equal temperament, range (-50, 50]
    /// (positive = sharp, negative = flat)
    pub cents_deviation: f32,
}

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
    /// Computed note info from the detected pitch (None if pitch_hz is None
    /// or out of MIDI range; always present when pitch_hz is a valid in-range
    /// frequency).
    pub note_info: Option<NoteInfo>,
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
            note_info: Some(NoteInfo {
                midi_note: 69,
                note_name: "A".to_string(),
                octave: 4,
                cents_deviation: 0.0,
            }),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AudioEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pitch_hz, Some(440.0));
        assert!(parsed.is_onset);
        assert!(parsed.note_info.is_some());
    }
}
