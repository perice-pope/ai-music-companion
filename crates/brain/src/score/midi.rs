//! MIDI parser — reads `.mid` / `.midi` files into a [`ScoreModel`].
//!
//! Uses `midly` for zero-copy MIDI parsing.

use midly::{Format, MidiMessage, Smf, TrackEventKind};

use super::{
    midi_to_hz, KeyMode, KeySignature, Measure, ScoreError, ScoreModel, ScoreNote, TimeSignature,
};

/// Parse raw MIDI bytes into a [`ScoreModel`].
pub fn parse_midi_bytes(bytes: &[u8]) -> Result<ScoreModel, ScoreError> {
    let smf = Smf::parse(bytes).map_err(|e| ScoreError::Midi(e.to_string()))?;

    let ticks_per_beat = match smf.header.timing {
        midly::Timing::Metrical(tpb) => tpb.as_int() as f64,
        midly::Timing::Timecode(fps, sub) => {
            // Fall back to a reasonable default; timecode-based files are rare
            // for sheet music. Use fps * sub as ticks-per-beat approximation.
            f64::from(fps.as_f32()) * f64::from(sub)
        }
    };

    let mut title = String::from("Untitled");
    let mut instrument: Option<String> = None;
    let mut time_signature = TimeSignature::default();
    let mut key_signature = KeySignature::default();
    let mut tempo_bpm: f64 = 120.0;

    // Collect all note events with absolute tick positions from all tracks
    let mut raw_notes: Vec<RawNote> = Vec::new();

    for (track_idx, track) in smf.tracks.iter().enumerate() {
        let mut abs_tick: u64 = 0;
        // Track active note-on events: (key, velocity, start_tick)
        let mut active_notes: Vec<(u8, u8, u64)> = Vec::new();

        for event in track {
            abs_tick += event.delta.as_int() as u64;

            match event.kind {
                TrackEventKind::Meta(meta) => {
                    match meta {
                        midly::MetaMessage::TrackName(name_bytes) => {
                            if let Ok(name) = std::str::from_utf8(name_bytes) {
                                let name = name.trim().to_string();
                                if !name.is_empty() {
                                    // Use first track name as title, subsequent as instrument
                                    if track_idx == 0
                                        || smf.header.format == Format::SingleTrack
                                    {
                                        title = name.clone();
                                    }
                                    if instrument.is_none() && track_idx > 0 {
                                        instrument = Some(name);
                                    }
                                }
                            }
                        }
                        midly::MetaMessage::Tempo(t) => {
                            // Microseconds per beat → BPM
                            let uspb = t.as_int() as f64;
                            if uspb > 0.0 {
                                tempo_bpm = 60_000_000.0 / uspb;
                            }
                        }
                        midly::MetaMessage::TimeSignature(num, denom_pow, _, _) => {
                            time_signature = TimeSignature {
                                beats: num,
                                beat_type: 1u8.checked_shl(denom_pow as u32).unwrap_or(4),
                            };
                        }
                        midly::MetaMessage::KeySignature(sf, minor) => {
                            key_signature = KeySignature {
                                fifths: sf,
                                mode: if minor {
                                    KeyMode::Minor
                                } else {
                                    KeyMode::Major
                                },
                            };
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Midi { message, .. } => match message {
                    MidiMessage::NoteOn { key, vel } => {
                        if vel.as_int() == 0 {
                            // Note-on with velocity 0 is treated as note-off
                            close_note(&mut active_notes, &mut raw_notes, key.as_int(), abs_tick);
                        } else {
                            active_notes.push((key.as_int(), vel.as_int(), abs_tick));
                        }
                    }
                    MidiMessage::NoteOff { key, .. } => {
                        close_note(&mut active_notes, &mut raw_notes, key.as_int(), abs_tick);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // Close any notes that were still active at end of track
        let final_tick = abs_tick;
        for (key, _vel, start) in active_notes.drain(..) {
            raw_notes.push(RawNote {
                midi_key: key,
                start_tick: start,
                duration_ticks: final_tick.saturating_sub(start),
            });
        }
    }

    // Sort by start_tick for deterministic measure assignment
    raw_notes.sort_by_key(|n| n.start_tick);

    // Convert raw notes into measures
    let beats_per_measure = time_signature.beats as f64;
    let ticks_per_measure = ticks_per_beat * beats_per_measure;
    let measures = build_measures(&raw_notes, ticks_per_beat, ticks_per_measure);

    Ok(ScoreModel {
        title,
        composer: None,
        instrument,
        time_signature,
        key_signature,
        tempo_bpm,
        measures,
    })
}

// ── Internal types and helpers ────────────────────────────────────────

struct RawNote {
    midi_key: u8,
    start_tick: u64,
    duration_ticks: u64,
}

/// Close the first matching active note and push a RawNote.
fn close_note(active: &mut Vec<(u8, u8, u64)>, out: &mut Vec<RawNote>, key: u8, off_tick: u64) {
    if let Some(idx) = active.iter().position(|(k, _, _)| *k == key) {
        let (midi_key, _vel, start_tick) = active.remove(idx);
        out.push(RawNote {
            midi_key,
            start_tick,
            duration_ticks: off_tick.saturating_sub(start_tick),
        });
    }
}

/// Group raw notes into measures.
fn build_measures(
    notes: &[RawNote],
    ticks_per_beat: f64,
    ticks_per_measure: f64,
) -> Vec<Measure> {
    if notes.is_empty() {
        return Vec::new();
    }

    let max_tick = notes
        .iter()
        .map(|n| n.start_tick + n.duration_ticks)
        .max()
        .unwrap_or(0);

    let num_measures = if ticks_per_measure > 0.0 {
        ((max_tick as f64) / ticks_per_measure).ceil() as usize
    } else {
        1
    };
    let num_measures = num_measures.max(1);

    let mut measures: Vec<Measure> = (0..num_measures)
        .map(|i| Measure {
            number: i + 1,
            notes: Vec::new(),
        })
        .collect();

    for note in notes {
        let measure_idx = if ticks_per_measure > 0.0 {
            (note.start_tick as f64 / ticks_per_measure).floor() as usize
        } else {
            0
        };
        let measure_idx = measure_idx.min(measures.len() - 1);

        let beat_in_measure = if ticks_per_beat > 0.0 {
            (note.start_tick as f64 - (measure_idx as f64 * ticks_per_measure)) / ticks_per_beat
        } else {
            0.0
        };

        let duration_beats = if ticks_per_beat > 0.0 {
            note.duration_ticks as f64 / ticks_per_beat
        } else {
            note.duration_ticks as f64
        };

        let hz = midi_to_hz(note.midi_key as f64);

        measures[measure_idx].notes.push(ScoreNote {
            pitch_hz: hz,
            midi_number: note.midi_key,
            duration_beats,
            start_beat: beat_in_measure,
            dynamic: None,
            is_rest: false,
        });
    }

    measures
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid MIDI file (format 0) with a C major scale.
    ///
    /// The file uses 480 ticks per beat, 4/4 time, 120 BPM, key of C major,
    /// and 8 quarter notes: C4, D4, E4, F4, G4, A4, B4, C5.
    fn build_c_major_scale_midi() -> Vec<u8> {
        let mut buf = Vec::new();

        // ── Header chunk ──
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&(6u32).to_be_bytes()); // chunk length
        buf.extend_from_slice(&(0u16).to_be_bytes()); // format 0
        buf.extend_from_slice(&(1u16).to_be_bytes()); // 1 track
        buf.extend_from_slice(&(480u16).to_be_bytes()); // 480 ticks/beat

        // ── Track chunk ──
        let mut track = Vec::new();

        // Track name: "C Major Scale"
        write_meta_event(&mut track, 0, 0x03, b"C Major Scale");

        // Tempo: 120 BPM = 500000 us/beat
        let tempo_bytes: [u8; 3] = [0x07, 0xA1, 0x20]; // 500000
        write_meta_event(&mut track, 0, 0x51, &tempo_bytes);

        // Time signature: 4/4, 24 MIDI clocks per click, 8 32nd-notes per beat
        write_meta_event(&mut track, 0, 0x58, &[4, 2, 24, 8]);

        // Key signature: C major (0 sharps/flats, major)
        write_meta_event(&mut track, 0, 0x59, &[0, 0]);

        // Notes: C4=60, D4=62, E4=64, F4=65, G4=67, A4=69, B4=71, C5=72
        let notes: &[u8] = &[60, 62, 64, 65, 67, 69, 71, 72];
        let velocity: u8 = 80;
        let note_duration: u16 = 480; // 1 beat

        for (i, &key) in notes.iter().enumerate() {
            let delta = if i == 0 { 0 } else { note_duration };
            // Note Off for previous note (except first)
            if i > 0 {
                write_midi_event(&mut track, delta, 0x80, notes[i - 1], 0);
            }
            // Note On
            write_midi_event(&mut track, if i == 0 { 0 } else { 0 }, 0x90, key, velocity);
        }
        // Note Off for last note
        write_midi_event(&mut track, note_duration, 0x80, *notes.last().unwrap(), 0);

        // End of track
        write_meta_event(&mut track, 0, 0x2F, &[]);

        buf.extend_from_slice(b"MTrk");
        buf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        buf.extend_from_slice(&track);

        buf
    }

    fn write_variable_length(buf: &mut Vec<u8>, mut value: u32) {
        let mut bytes = Vec::new();
        bytes.push((value & 0x7F) as u8);
        value >>= 7;
        while value > 0 {
            bytes.push((value & 0x7F) as u8 | 0x80);
            value >>= 7;
        }
        bytes.reverse();
        buf.extend_from_slice(&bytes);
    }

    fn write_meta_event(buf: &mut Vec<u8>, delta: u16, meta_type: u8, data: &[u8]) {
        write_variable_length(buf, delta as u32);
        buf.push(0xFF);
        buf.push(meta_type);
        write_variable_length(buf, data.len() as u32);
        buf.extend_from_slice(data);
    }

    fn write_midi_event(buf: &mut Vec<u8>, delta: u16, status: u8, data1: u8, data2: u8) {
        write_variable_length(buf, delta as u32);
        buf.push(status);
        buf.push(data1);
        buf.push(data2);
    }

    #[test]
    fn parse_midi_extracts_notes() {
        let midi_bytes = build_c_major_scale_midi();
        let model = parse_midi_bytes(&midi_bytes).expect("parse MIDI");

        let total_notes: usize = model.measures.iter().map(|m| m.notes.len()).sum();
        assert_eq!(total_notes, 8, "Expected 8 notes, got {total_notes}");

        // Collect all MIDI note numbers across measures
        let midi_numbers: Vec<u8> = model
            .measures
            .iter()
            .flat_map(|m| m.notes.iter().map(|n| n.midi_number))
            .collect();
        assert_eq!(
            midi_numbers,
            vec![60, 62, 64, 65, 67, 69, 71, 72],
            "MIDI note numbers should match C major scale"
        );
    }

    #[test]
    fn parse_midi_extracts_tempo() {
        let midi_bytes = build_c_major_scale_midi();
        let model = parse_midi_bytes(&midi_bytes).expect("parse MIDI");
        assert!(
            (model.tempo_bpm - 120.0).abs() < 0.1,
            "Tempo should be 120 BPM, got {}",
            model.tempo_bpm
        );
    }

    #[test]
    fn parse_midi_extracts_time_and_key_signatures() {
        let midi_bytes = build_c_major_scale_midi();
        let model = parse_midi_bytes(&midi_bytes).expect("parse MIDI");

        assert_eq!(model.time_signature.beats, 4);
        assert_eq!(model.time_signature.beat_type, 4);
        assert_eq!(model.key_signature.fifths, 0);
        assert_eq!(model.key_signature.mode, KeyMode::Major);
    }

    #[test]
    fn parse_midi_extracts_title() {
        let midi_bytes = build_c_major_scale_midi();
        let model = parse_midi_bytes(&midi_bytes).expect("parse MIDI");
        assert_eq!(model.title, "C Major Scale");
    }

    #[test]
    fn parse_midi_handles_empty_track() {
        let mut buf = Vec::new();
        // Header: format 0, 1 track, 480 tpb
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&480u16.to_be_bytes());

        // Track with only end-of-track
        let mut track = Vec::new();
        write_meta_event(&mut track, 0, 0x2F, &[]);

        buf.extend_from_slice(b"MTrk");
        buf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        buf.extend_from_slice(&track);

        let model = parse_midi_bytes(&buf).expect("parse empty-track MIDI");
        assert!(
            model.measures.is_empty(),
            "Empty track should produce no measures"
        );
    }

    #[test]
    fn parse_rejects_invalid_midi() {
        let garbage = b"This is definitely not a MIDI file!!!";
        let result = parse_midi_bytes(garbage);
        assert!(result.is_err(), "Garbage bytes should return an error");
        assert!(
            matches!(result.unwrap_err(), ScoreError::Midi(_)),
            "Error should be Midi variant"
        );
    }

    #[test]
    fn note_durations_are_one_beat() {
        let midi_bytes = build_c_major_scale_midi();
        let model = parse_midi_bytes(&midi_bytes).expect("parse MIDI");

        for measure in &model.measures {
            for note in &measure.notes {
                assert!(
                    (note.duration_beats - 1.0).abs() < 0.01,
                    "Each note should be 1 beat, got {}",
                    note.duration_beats
                );
            }
        }
    }
}
