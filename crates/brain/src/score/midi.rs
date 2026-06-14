//! MIDI parser — reads `.mid` / `.midi` files into a [`ScoreModel`].
//!
//! Uses `midly` for zero-copy MIDI parsing.

use midly::{Format, MidiMessage, Smf, TrackEventKind};

use super::quantize::{quantize_notes, QuantizeConfig, QuantizedEvent, RawNoteEvent};
use super::{
    midi_to_hz, KeyMode, KeySignature, Measure, ScoreError, ScoreModel, ScoreNote, TimeSignature,
};

/// Parse raw MIDI bytes into a [`ScoreModel`].
pub fn parse_midi_bytes(bytes: &[u8]) -> Result<ScoreModel, ScoreError> {
    let smf = Smf::parse(bytes).map_err(|e| ScoreError::Midi(e.to_string()))?;

    // `ticks_per_quarter` is the number of MIDI ticks per quarter note, which
    // is what MIDI's Metrical timing natively expresses. We convert to
    // "ticks per beat" below once we know the time signature, because in a
    // 3/8 or 6/8 meter the *beat* is an eighth note, not a quarter.
    let ticks_per_quarter = match smf.header.timing {
        midly::Timing::Metrical(tpb) => tpb.as_int() as f64,
        midly::Timing::Timecode(_, _) => {
            // Timecode-based MIDI expresses absolute time (frames-per-second
            // × subframes), not metrical beats. There is no correct conversion
            // to ticks-per-quarter without additional tempo/meter hints, and
            // timecode files are overwhelmingly used for film scoring sync,
            // not practice pieces. Reject explicitly rather than silently
            // producing wrong beat/measure positions.
            return Err(ScoreError::Midi(
                "timecode-based MIDI timing is not supported; \
                 please export with metrical (ticks-per-quarter) timing"
                    .to_string(),
            ));
        }
    };

    // Reject SMF Format 2 (sequential tracks that play as independent
    // pieces, one after another). Our single-timeline merging logic below
    // would incorrectly overlap them as if they were simultaneous.
    // Format 2 is rare outside of drum machine files — supporting it would
    // mean producing multiple ScoreModels or concatenating tracks with
    // track-local abs_tick offsets, neither of which fits the current API.
    if smf.header.format == Format::Sequential {
        return Err(ScoreError::Midi(
            "SMF Format 2 (sequential multi-song) is not supported; \
             please provide a Format 0 (single track) or Format 1 \
             (simultaneous multi-track) file"
                .to_string(),
        ));
    }

    let mut title = String::from("Untitled");
    let mut instrument: Option<String> = None;
    let mut time_signature = TimeSignature::default();
    let mut key_signature = KeySignature::default();
    // Tempo is tracked internally in quarter-note BPM (because MIDI's
    // `MetaMessage::Tempo` is defined as microseconds-per-quarter-note,
    // independent of any time signature). We convert to signature-beat
    // BPM at the end so downstream consumers using `time_signature.beat_type`
    // see consistent units.
    let mut tempo_quarter_bpm: f64 = 120.0;

    // Collect all note events with absolute tick positions from all tracks
    let mut raw_notes: Vec<RawNote> = Vec::new();

    for track in smf.tracks.iter() {
        let mut abs_tick: u64 = 0;
        // Track active note-on events: (channel, key, velocity, start_tick).
        // Channel is included so that the same MIDI key sounding on two
        // different channels does not cross-match in `close_note`.
        let mut active_notes: Vec<(u8, u8, u8, u64)> = Vec::new();

        for event in track {
            abs_tick += event.delta.as_int() as u64;

            match event.kind {
                TrackEventKind::Meta(meta) => apply_meta_message(
                    meta,
                    &mut title,
                    &mut instrument,
                    &mut time_signature,
                    &mut key_signature,
                    &mut tempo_quarter_bpm,
                ),
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } => {
                            if vel.as_int() == 0 {
                                // Note-on with velocity 0 is treated as note-off
                                close_note(
                                    &mut active_notes,
                                    &mut raw_notes,
                                    ch,
                                    key.as_int(),
                                    abs_tick,
                                );
                            } else {
                                active_notes.push((ch, key.as_int(), vel.as_int(), abs_tick));
                            }
                        }
                        MidiMessage::NoteOff { key, .. } => {
                            close_note(
                                &mut active_notes,
                                &mut raw_notes,
                                ch,
                                key.as_int(),
                                abs_tick,
                            );
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Close any notes that were still active at end of track
        let final_tick = abs_tick;
        for (_ch, key, _vel, start) in active_notes.drain(..) {
            raw_notes.push(RawNote {
                midi_key: key,
                start_tick: start,
                duration_ticks: final_tick.saturating_sub(start),
            });
        }
    }

    // Sort by start_tick for deterministic measure assignment
    raw_notes.sort_by_key(|n| n.start_tick);

    // Convert ticks-per-quarter into ticks-per-beat using the time signature's
    // beat unit. In 6/8 the beat is an eighth note (beat_type = 8), so
    // ticks_per_beat = ticks_per_quarter * (4 / 8) = half of a quarter.
    let beat_type = time_signature.beat_type.max(1) as f64;
    let ticks_per_beat = ticks_per_quarter * (4.0 / beat_type);
    let beats_per_measure = time_signature.beats as f64;
    let ticks_per_measure = ticks_per_beat * beats_per_measure;

    // Quantize raw performance/transcription timing onto a rhythmic grid and
    // fill gaps with rests *before* notating. Without this, fractional onsets
    // and durations (e.g. a 0.97-beat "quarter") render as garbage rhythms.
    // See `super::quantize`. Quantization is offline, deterministic and pure.
    let raw_events: Vec<RawNoteEvent> = raw_notes
        .iter()
        .map(|n| RawNoteEvent {
            midi_key: n.midi_key,
            start_tick: n.start_tick,
            duration_ticks: n.duration_ticks,
        })
        .collect();
    let quantized = quantize_notes(
        &raw_events,
        ticks_per_beat,
        ticks_per_measure,
        &QuantizeConfig::default(),
    )
    .map_err(|e| ScoreError::Midi(e.to_string()))?;

    let measures = build_measures(&quantized.events, ticks_per_beat, ticks_per_measure);

    // Convert quarter-note BPM to signature-beat BPM so downstream consumers
    // using `duration_beats` (which is in signature-beat units) see consistent
    // tempo math. In 6/8: one quarter = 2 eighths, so quarter_bpm=120 means
    // eighth_bpm=240. Formula: tempo_bpm = quarter_bpm * (beat_type / 4).
    // This is the inverse of the ticks_per_beat multiplier above — ticks
    // scale with beat *period*, tempo scales with beat *frequency*.
    let tempo_bpm = tempo_quarter_bpm * (beat_type / 4.0);

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

/// Close the first matching active note (same channel AND key) and push a RawNote.
/// Apply a MIDI `Meta` event to the running header state: title, instrument,
/// tempo, time signature, key signature. Unhandled meta types are ignored.
///
/// Title is the first non-empty `TrackName` seen on any track (Format-1 files
/// often put the real title on track 1, not track 0); the instrument is the
/// first track-name that isn't the title.
fn apply_meta_message(
    meta: midly::MetaMessage,
    title: &mut String,
    instrument: &mut Option<String>,
    time_signature: &mut TimeSignature,
    key_signature: &mut KeySignature,
    tempo_quarter_bpm: &mut f64,
) {
    match meta {
        midly::MetaMessage::TrackName(name_bytes) => {
            if let Ok(name) = std::str::from_utf8(name_bytes) {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    if *title == "Untitled" {
                        *title = name.clone();
                    }
                    if instrument.is_none() && *title != name {
                        *instrument = Some(name);
                    }
                }
            }
        }
        midly::MetaMessage::Tempo(t) => {
            // Microseconds per quarter-note → quarter-note BPM.
            let uspqn = t.as_int() as f64;
            if uspqn > 0.0 {
                *tempo_quarter_bpm = 60_000_000.0 / uspqn;
            }
        }
        midly::MetaMessage::TimeSignature(num, denom_pow, _, _) => {
            *time_signature = TimeSignature {
                beats: num,
                beat_type: 1u8.checked_shl(denom_pow as u32).unwrap_or(4),
            };
        }
        midly::MetaMessage::KeySignature(sf, minor) => {
            *key_signature = KeySignature {
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

fn close_note(
    active: &mut Vec<(u8, u8, u8, u64)>,
    out: &mut Vec<RawNote>,
    channel: u8,
    key: u8,
    off_tick: u64,
) {
    if let Some(idx) = active
        .iter()
        .position(|(c, k, _, _)| *c == channel && *k == key)
    {
        let (_ch, midi_key, _vel, start_tick) = active.remove(idx);
        out.push(RawNote {
            midi_key,
            start_tick,
            duration_ticks: off_tick.saturating_sub(start_tick),
        });
    }
}

/// Group quantized events (notes and rests) into measures.
///
/// Events arrive grid-snapped and gap-filled from [`super::quantize`], so this
/// stage only has to assign each event to its measure, compute its in-measure
/// `start_beat`, and convert tick durations to beats.
fn build_measures(
    events: &[QuantizedEvent],
    ticks_per_beat: f64,
    ticks_per_measure: f64,
) -> Vec<Measure> {
    if events.is_empty() {
        return Vec::new();
    }

    let max_tick = events
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

    for event in events {
        let measure_idx = if ticks_per_measure > 0.0 {
            (event.start_tick as f64 / ticks_per_measure).floor() as usize
        } else {
            0
        };
        let measure_idx = measure_idx.min(measures.len() - 1);

        let beat_in_measure = if ticks_per_beat > 0.0 {
            (event.start_tick as f64 - (measure_idx as f64 * ticks_per_measure)) / ticks_per_beat
        } else {
            0.0
        };

        let duration_beats = if ticks_per_beat > 0.0 {
            event.duration_ticks as f64 / ticks_per_beat
        } else {
            event.duration_ticks as f64
        };

        let (pitch_hz, midi_number) = if event.is_rest {
            (0.0, 0)
        } else {
            (midi_to_hz(event.midi_key as f64), event.midi_key)
        };

        measures[measure_idx].notes.push(ScoreNote {
            pitch_hz,
            midi_number,
            duration_beats,
            start_beat: beat_in_measure,
            dynamic: None,
            is_rest: event.is_rest,
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
            write_midi_event(&mut track, 0, 0x90, key, velocity);
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

    /// Build a MIDI file in 6/8 time with six eighth-note C4 notes (one full
    /// measure). 480 ticks-per-quarter means 240 ticks per eighth note (the
    /// beat in 6/8).
    fn build_six_eight_midi() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&480u16.to_be_bytes());

        let mut track = Vec::new();
        // Tempo: 120 BPM
        write_meta_event(&mut track, 0, 0x51, &[0x07, 0xA1, 0x20]);
        // Time signature 6/8: numerator=6, denom_pow=3 (2^3 = 8), clocks=24, 32nds=8
        write_meta_event(&mut track, 0, 0x58, &[6, 3, 24, 8]);

        // Six eighth-notes of C4 (MIDI 60). Each eighth = 240 ticks.
        let eighth: u16 = 240;
        for _ in 0..6 {
            write_midi_event(&mut track, 0, 0x90, 60, 80);
            write_midi_event(&mut track, eighth, 0x80, 60, 0);
        }

        write_meta_event(&mut track, 0, 0x2F, &[]);

        buf.extend_from_slice(b"MTrk");
        buf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        buf.extend_from_slice(&track);
        buf
    }

    #[test]
    fn six_eight_time_uses_eighth_note_beats() {
        let midi_bytes = build_six_eight_midi();
        let model = parse_midi_bytes(&midi_bytes).expect("parse 6/8 MIDI");

        assert_eq!(model.time_signature.beats, 6);
        assert_eq!(model.time_signature.beat_type, 8);

        // Expect exactly one measure with six notes (six eighth-note beats).
        assert_eq!(
            model.measures.len(),
            1,
            "6/8 with six eighths should fit in one measure, got {} measures",
            model.measures.len()
        );
        let notes = &model.measures[0].notes;
        assert_eq!(notes.len(), 6, "Expected 6 eighth notes");

        // Each eighth note should register as exactly 1 beat in 6/8 time.
        for (i, note) in notes.iter().enumerate() {
            assert!(
                (note.duration_beats - 1.0).abs() < 0.01,
                "Note {i} duration in 6/8 should be 1 beat (an eighth), got {}",
                note.duration_beats
            );
            assert!(
                (note.start_beat - i as f64).abs() < 0.01,
                "Note {i} start_beat should be {i}.0, got {}",
                note.start_beat
            );
        }
    }

    /// Build a MIDI file where the same key (C4) is held on two different
    /// channels. Channel 0's NoteOn comes FIRST but its NoteOff comes SECOND,
    /// so a non-channel-aware matcher would close channel 0's note at
    /// channel 1's NoteOff tick (the buggy case), producing the wrong
    /// durations.
    ///
    /// Timeline:
    ///   t=0     NoteOn  ch0 C4
    ///   t=240   NoteOn  ch1 C4
    ///   t=480   NoteOff ch1 C4   (ch1 duration = 240 ticks = 0.5 beats)
    ///   t=960   NoteOff ch0 C4   (ch0 duration = 960 ticks = 2.0 beats)
    ///
    /// Buggy behavior (match first active by key only):
    ///   t=480 closes ch0 (wrong) -> duration = 480 ticks = 1.0 beat
    ///   t=960 closes ch1 -> duration = 720 ticks = 1.5 beats
    /// So the buggy output is [1.0, 1.5] and the correct output is [0.5, 2.0].
    fn build_two_channel_overlap_midi() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&480u16.to_be_bytes());

        let mut track = Vec::new();
        write_meta_event(&mut track, 0, 0x51, &[0x07, 0xA1, 0x20]); // 120 BPM
        write_meta_event(&mut track, 0, 0x58, &[4, 2, 24, 8]); // 4/4

        // t=0: NoteOn C4 on channel 0 (status 0x90)
        write_midi_event(&mut track, 0, 0x90, 60, 80);
        // t=240: NoteOn C4 on channel 1 (status 0x91), delta from previous = 240
        write_midi_event(&mut track, 240, 0x91, 60, 80);
        // t=480: NoteOff C4 on channel 1 (status 0x81), delta = 240
        write_midi_event(&mut track, 240, 0x81, 60, 0);
        // t=960: NoteOff C4 on channel 0 (status 0x80), delta = 480
        write_midi_event(&mut track, 480, 0x80, 60, 0);
        write_meta_event(&mut track, 0, 0x2F, &[]);

        buf.extend_from_slice(b"MTrk");
        buf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        buf.extend_from_slice(&track);
        buf
    }

    #[test]
    fn same_key_on_different_channels_does_not_cross_match() {
        let midi_bytes = build_two_channel_overlap_midi();
        let model = parse_midi_bytes(&midi_bytes).expect("parse two-channel MIDI");

        // Filter to sounding notes: quantization inserts rests to fill gaps
        // (e.g. the tail of the measure after the 2-beat note), so the measure
        // now contains rests in addition to the two sounding notes.
        let mut durations: Vec<f64> = model
            .measures
            .iter()
            .flat_map(|m| {
                m.notes
                    .iter()
                    .filter(|n| !n.is_rest)
                    .map(|n| n.duration_beats)
            })
            .collect();
        assert_eq!(durations.len(), 2, "Expected 2 sounding notes");

        durations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // With the fix: ch1 lasts 0.5 beats, ch0 lasts 2.0 beats.
        // Without the fix (buggy): the durations would be [1.0, 1.5].
        assert!(
            (durations[0] - 0.5).abs() < 0.01,
            "Shortest note (ch1) should be 0.5 beats, got {} (all: {:?})",
            durations[0],
            durations,
        );
        assert!(
            (durations[1] - 2.0).abs() < 0.01,
            "Longest note (ch0) should be 2.0 beats, got {} (all: {:?})",
            durations[1],
            durations,
        );
    }

    /// Build a minimal MIDI file with timecode-based timing (SMPTE 24 fps)
    /// just to trigger the Timing::Timecode branch in parse_midi_bytes.
    fn build_timecode_midi() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // format 0
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // 1 track
                                                       // SMPTE: high bit set on fps byte. -24 fps + 40 subframes => 0xE8, 0x28
        bytes.extend_from_slice(&[0xE8, 0x28]);
        // Minimal empty track with End-of-Track meta event
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&4_u32.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        bytes
    }

    #[test]
    fn timecode_timing_is_rejected_with_clear_error() {
        // Regression: we previously fell back to `fps * sub` as a pseudo
        // ticks-per-quarter, which is ticks-per-second in disguise and produces
        // badly-wrong beat positions. Reject explicitly instead.
        let bytes = build_timecode_midi();
        let result = parse_midi_bytes(&bytes);
        match result {
            Err(ScoreError::Midi(msg)) => {
                assert!(
                    msg.to_lowercase().contains("timecode"),
                    "Error should mention timecode, got: {msg}"
                );
            }
            Err(other) => panic!("Expected Midi error, got: {other:?}"),
            Ok(_) => panic!("Timecode-based MIDI should be rejected, not parsed"),
        }
    }

    /// Minimal Format-2 (Sequential) MIDI file with two tiny tracks.
    fn build_format_2_midi() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6_u32.to_be_bytes());
        bytes.extend_from_slice(&2_u16.to_be_bytes()); // format 2
        bytes.extend_from_slice(&2_u16.to_be_bytes()); // 2 tracks
        bytes.extend_from_slice(&480_u16.to_be_bytes()); // 480 tpq
        for _ in 0..2 {
            bytes.extend_from_slice(b"MTrk");
            bytes.extend_from_slice(&4_u32.to_be_bytes());
            bytes.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        }
        bytes
    }

    #[test]
    fn format_2_midi_is_rejected_with_clear_error() {
        // Regression: Format 2 files define sequential (not simultaneous)
        // tracks. Merging them into a single timeline produces garbage
        // measure positions; we require Format 0 or 1.
        let bytes = build_format_2_midi();
        let err = parse_midi_bytes(&bytes).unwrap_err();
        let msg = match &err {
            ScoreError::Midi(m) => m.clone(),
            other => panic!("Expected Midi error, got: {other:?}"),
        };
        assert!(
            msg.contains("Format 2"),
            "Error should mention Format 2, got: {msg}"
        );
    }

    /// Build a Format-1 MIDI file where track 0 has tempo + no TrackName,
    /// and track 1 carries the first non-empty TrackName ("My Piece").
    fn build_format_1_name_on_track_1() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6_u32.to_be_bytes());
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // format 1
        bytes.extend_from_slice(&2_u16.to_be_bytes()); // 2 tracks
        bytes.extend_from_slice(&480_u16.to_be_bytes());

        // Track 0: tempo + end-of-track (no TrackName)
        let mut t0 = Vec::new();
        // Tempo 500000 us/qn → 120 BPM
        write_meta_event(&mut t0, 0, 0x51, &[0x07, 0xA1, 0x20]);
        write_meta_event(&mut t0, 0, 0x2F, &[]);
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(t0.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&t0);

        // Track 1: TrackName "My Piece" + a single note + end-of-track
        let mut t1 = Vec::new();
        write_meta_event(&mut t1, 0, 0x03, b"My Piece");
        write_midi_event(&mut t1, 0, 0x90, 60, 80);
        write_midi_event(&mut t1, 480, 0x80, 60, 0);
        write_meta_event(&mut t1, 0, 0x2F, &[]);
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(t1.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&t1);

        bytes
    }

    #[test]
    fn title_uses_first_non_empty_track_name_not_only_track_zero() {
        // Regression: Format-1 files often have a meta-only track 0; the
        // real piece title shows up as TrackName on track 1. Previously
        // we only looked at track 0, so this file came back as "Untitled".
        let bytes = build_format_1_name_on_track_1();
        let model = parse_midi_bytes(&bytes).expect("parse Format 1 MIDI");
        assert_eq!(
            model.title, "My Piece",
            "First non-empty TrackName across all tracks should become the title"
        );
    }

    #[test]
    fn tempo_is_in_signature_beat_units_not_quarter_notes() {
        // Regression: MIDI tempo is always quarter-note-based. Our model's
        // `tempo_bpm` is expressed in time-signature-beat units (so
        // downstream duration math stays consistent). In 6/8 with
        // quarter=120, we expect eighth=240 BPM at the model level.
        let bytes = build_six_eight_midi();
        let model = parse_midi_bytes(&bytes).expect("parse 6/8 MIDI");
        assert_eq!(model.time_signature.beats, 6);
        assert_eq!(model.time_signature.beat_type, 8);
        assert!(
            (model.tempo_bpm - 240.0).abs() < 0.01,
            "Expected 240 eighth-BPM (= 120 quarter-BPM × 2) in 6/8, got {}",
            model.tempo_bpm,
        );
    }
}
