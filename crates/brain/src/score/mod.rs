//! Score parsing — reads MusicXML and MIDI files into a unified [`ScoreModel`].
//!
//! This module provides [`ScoreParser`] which auto-detects the format from
//! the file extension and delegates to the appropriate parser. The inverse
//! direction (serialising a `ScoreModel` back to MusicXML) lives in
//! [`emit`] and is exposed via [`score_model_to_musicxml`].

pub mod emit;
pub mod midi;
pub mod musicxml;
pub mod quantize;

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ── Errors ────────────────────────────────────────────────────────────

/// Errors that can occur when parsing a score file.
#[derive(Debug, Error)]
pub enum ScoreError {
    #[error("unsupported file extension: {0}")]
    UnsupportedFormat(String),
    #[error("I/O error reading score file: {0}")]
    Io(#[from] std::io::Error),
    #[error("MusicXML parse error: {0}")]
    MusicXml(String),
    #[error("MIDI parse error: {0}")]
    Midi(String),
    #[error("part index {0} out of range")]
    PartNotFound(usize),
}

// ── Score model types ─────────────────────────────────────────────────

/// Complete score model produced by parsing a sheet music file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreModel {
    pub title: String,
    pub composer: Option<String>,
    pub instrument: Option<String>,
    pub time_signature: TimeSignature,
    pub key_signature: KeySignature,
    pub tempo_bpm: f64,
    pub measures: Vec<Measure>,
}

/// Time signature (e.g. 4/4, 3/4, 6/8).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeSignature {
    /// Number of beats per measure.
    pub beats: u8,
    /// Beat unit denominator (4 = quarter note, 8 = eighth note, etc.).
    pub beat_type: u8,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self {
            beats: 4,
            beat_type: 4,
        }
    }
}

/// Key signature expressed as position on the circle of fifths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeySignature {
    /// -7 (7 flats) to +7 (7 sharps). 0 = C major / A minor.
    pub fifths: i8,
    /// Major or minor mode.
    pub mode: KeyMode,
}

impl Default for KeySignature {
    fn default() -> Self {
        Self {
            fifths: 0,
            mode: KeyMode::Major,
        }
    }
}

/// Major or minor key mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyMode {
    Major,
    Minor,
}

/// A single measure (bar) containing notes and rests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measure {
    /// 1-based measure number.
    pub number: usize,
    /// Notes and rests in this measure, ordered by `start_beat`.
    pub notes: Vec<ScoreNote>,
}

/// A single note or rest within a measure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreNote {
    /// Frequency in Hz (0.0 for rests).
    pub pitch_hz: f64,
    /// MIDI note number (0 for rests).
    pub midi_number: u8,
    /// Duration expressed in beats (e.g. 1.0 = quarter note in 4/4).
    pub duration_beats: f64,
    /// Start position within the measure in beats (0-based).
    pub start_beat: f64,
    /// Dynamic marking, if present.
    pub dynamic: Option<Dynamic>,
    /// Whether this is a rest rather than a sounding note.
    pub is_rest: bool,
}

/// Dynamic marking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum Dynamic {
    PPP,
    PP,
    P,
    MP,
    MF,
    F,
    FF,
    FFF,
}

// ── Pitch conversion helpers ──────────────────────────────────────────

/// Convert a pitch name (step, octave, chromatic alteration) to Hz.
///
/// Uses A4 = 440 Hz equal temperament.
///
/// - `step`: one of `"C"`, `"D"`, `"E"`, `"F"`, `"G"`, `"A"`, `"B"`
/// - `octave`: scientific pitch octave (4 = middle C octave)
/// - `alter`: chromatic alteration in semitones (1.0 = sharp, -1.0 = flat)
pub fn pitch_to_hz(step: &str, octave: i8, alter: f64) -> f64 {
    let midi = pitch_to_midi(step, octave, alter);
    midi_to_hz(midi)
}

/// Convert a pitch name to a (possibly fractional) MIDI note number.
pub fn pitch_to_midi(step: &str, octave: i8, alter: f64) -> f64 {
    let semitone_offset: f64 = match step {
        "C" => 0.0,
        "D" => 2.0,
        "E" => 4.0,
        "F" => 5.0,
        "G" => 7.0,
        "A" => 9.0,
        "B" => 11.0,
        _ => 0.0,
    };
    // MIDI: C4 = 60, so C(-1) = 0.
    12.0 * (octave as f64 + 1.0) + semitone_offset + alter
}

/// Convert a MIDI note number to Hz (A4 = 440 Hz).
pub fn midi_to_hz(midi: f64) -> f64 {
    440.0 * 2.0_f64.powf((midi - 69.0) / 12.0)
}

// ── ScoreParser facade ────────────────────────────────────────────────

/// Facade for parsing score files in any supported format.
pub struct ScoreParser;

impl ScoreParser {
    /// Parse a MusicXML file. Defaults to the first `<part>` (index 0).
    ///
    /// Use [`ScoreParser::parse_musicxml_part`] to select a specific part and
    /// [`ScoreParser::list_musicxml_parts`] to enumerate available part names.
    pub fn parse_musicxml(path: &Path) -> Result<ScoreModel, ScoreError> {
        Self::parse_musicxml_part(path, 0)
    }

    /// Parse a specific part from a multi-part MusicXML file.
    ///
    /// `part_index` is zero-based against the order of `<part>` elements in
    /// the file. Returns [`ScoreError::PartNotFound`] if the index is out of
    /// range.
    pub fn parse_musicxml_part(path: &Path, part_index: usize) -> Result<ScoreModel, ScoreError> {
        let xml = std::fs::read_to_string(path)?;
        musicxml::parse_musicxml_str_part(&xml, part_index)
    }

    /// List the part names (from `<part-name>` in `<part-list>`) in a
    /// MusicXML file, in the order the parts appear. Useful for letting a
    /// user pick which instrument to practice.
    pub fn list_musicxml_parts(path: &Path) -> Result<Vec<String>, ScoreError> {
        let xml = std::fs::read_to_string(path)?;
        musicxml::list_parts(&xml)
    }

    /// Parse a MIDI file.
    pub fn parse_midi(path: &Path) -> Result<ScoreModel, ScoreError> {
        let bytes = std::fs::read(path)?;
        midi::parse_midi_bytes(&bytes)
    }

    /// Auto-detect format from extension and parse.
    ///
    /// Supported extensions: `.musicxml`, `.xml`, `.mid`, `.midi`.
    /// Note: `.mxl` (compressed MusicXML) is not yet supported — it requires
    /// a ZIP-decompression dependency that we haven't taken on yet.
    pub fn parse(path: &Path) -> Result<ScoreModel, ScoreError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "musicxml" | "xml" => Self::parse_musicxml(path),
            "mid" | "midi" => Self::parse_midi(path),
            other => Err(ScoreError::UnsupportedFormat(other.to_string())),
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_auto_detects_musicxml_extension() {
        // Passing a non-existent file will fail with Io, not UnsupportedFormat.
        let path = PathBuf::from("/tmp/nonexistent.musicxml");
        let err = ScoreParser::parse(&path).unwrap_err();
        // The error should be Io (file not found), proving it routed to MusicXML.
        assert!(
            matches!(err, ScoreError::Io(_)),
            "Expected Io error for .musicxml, got: {err}"
        );
    }

    #[test]
    fn parse_auto_detects_midi_extension() {
        let path = PathBuf::from("/tmp/nonexistent.mid");
        let err = ScoreParser::parse(&path).unwrap_err();
        assert!(
            matches!(err, ScoreError::Io(_)),
            "Expected Io error for .mid, got: {err}"
        );
    }

    #[test]
    fn parse_rejects_unsupported_extension() {
        let path = PathBuf::from("/tmp/score.pdf");
        let err = ScoreParser::parse(&path).unwrap_err();
        assert!(
            matches!(err, ScoreError::UnsupportedFormat(ref ext) if ext == "pdf"),
            "Expected UnsupportedFormat(\"pdf\"), got: {err}"
        );
    }

    #[test]
    fn parse_rejects_mxl_extension() {
        // .mxl (compressed/zipped MusicXML) is not yet supported — reading
        // the file as text would yield garbage. Make sure auto-detect rejects
        // it with UnsupportedFormat rather than silently failing deep in the
        // XML parser.
        let path = PathBuf::from("/tmp/score.mxl");
        let err = ScoreParser::parse(&path).unwrap_err();
        assert!(
            matches!(err, ScoreError::UnsupportedFormat(ref ext) if ext == "mxl"),
            "Expected UnsupportedFormat(\"mxl\"), got: {err}"
        );
    }

    #[test]
    fn all_types_are_serializable() {
        let model = ScoreModel {
            title: "Test".to_string(),
            composer: Some("Bach".to_string()),
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            measures: vec![Measure {
                number: 1,
                notes: vec![ScoreNote {
                    pitch_hz: 440.0,
                    midi_number: 69,
                    duration_beats: 1.0,
                    start_beat: 0.0,
                    dynamic: Some(Dynamic::MF),
                    is_rest: false,
                }],
            }],
        };

        let json = serde_json::to_string(&model).expect("serialize ScoreModel");
        let roundtrip: ScoreModel = serde_json::from_str(&json).expect("deserialize ScoreModel");

        assert_eq!(roundtrip.title, "Test");
        assert_eq!(roundtrip.composer.as_deref(), Some("Bach"));
        assert_eq!(roundtrip.measures.len(), 1);
        assert_eq!(roundtrip.measures[0].notes.len(), 1);
        assert_eq!(roundtrip.measures[0].notes[0].midi_number, 69);
        assert!((roundtrip.tempo_bpm - 120.0).abs() < f64::EPSILON);
    }

    #[test]
    fn pitch_to_hz_a4_is_440() {
        let hz = pitch_to_hz("A", 4, 0.0);
        assert!((hz - 440.0).abs() < 0.01, "A4 should be 440 Hz, got {hz}");
    }

    #[test]
    fn pitch_to_hz_c4_is_261() {
        let hz = pitch_to_hz("C", 4, 0.0);
        assert!(
            (hz - 261.63).abs() < 0.1,
            "C4 should be ~261.63 Hz, got {hz}"
        );
    }

    #[test]
    fn pitch_to_hz_with_accidental() {
        // F#4 = MIDI 66 = 369.99 Hz
        let hz = pitch_to_hz("F", 4, 1.0);
        assert!(
            (hz - 369.99).abs() < 0.1,
            "F#4 should be ~369.99 Hz, got {hz}"
        );

        // Bb4 = MIDI 70 = 466.16 Hz
        let hz_flat = pitch_to_hz("B", 4, -1.0);
        assert!(
            (hz_flat - 466.16).abs() < 0.1,
            "Bb4 should be ~466.16 Hz, got {hz_flat}"
        );
    }

    #[test]
    fn midi_to_hz_boundaries() {
        // MIDI 0 = C(-1) ≈ 8.18 Hz
        let low = midi_to_hz(0.0);
        assert!(
            (low - 8.176).abs() < 0.01,
            "MIDI 0 should be ~8.18 Hz, got {low}"
        );

        // MIDI 127 = G9 ≈ 12543.85 Hz
        let high = midi_to_hz(127.0);
        assert!(
            (high - 12543.85).abs() < 1.0,
            "MIDI 127 should be ~12543.85 Hz, got {high}"
        );
    }
}
