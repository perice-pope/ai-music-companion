//! MIDI parser — reads `.mid` / `.midi` files into a [`ScoreModel`].
//!
//! Uses `midly` for zero-copy MIDI parsing.

use midly::{Format, MidiMessage, Smf, TrackEventKind};

use super::quantize::{quantize_notes, QuantizeConfig, QuantizedEvent, RawNoteEvent};
use super::{
    midi_to_hz, KeyMode, KeySignature, Measure, ScoreError, ScoreModel, ScoreNote, TimeSignature,
};

/// MIDI channel reserved for percussion in General MIDI (0-indexed 9,
/// "channel 10"). Drum hits carry note numbers that are kit-piece ids, not
/// pitches — merged into a melody they render as garbage notes, so note
/// events on this channel are skipped everywhere (#337 S1).
const PERCUSSION_CHANNEL: u8 = 9;

/// One playable track of a multi-track MIDI file, for the import part picker
/// (#337 S1) — the MIDI equivalent of MusicXML's `list_score_parts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiPartInfo {
    /// Index into the file's ORIGINAL track list — pass back to
    /// [`parse_midi_bytes_track`] to import just this track.
    pub track_index: usize,
    /// The track's `TrackName`, or `"Track N"` (1-based) when unnamed.
    pub name: String,
    /// Sounding (non-percussion) notes on the track.
    pub note_count: usize,
}

/// List the playable tracks of a MIDI file: every track with at least one
/// non-percussion note. Meta-only conductor tracks and pure drum tracks are
/// omitted — they aren't practice material on their own.
pub fn list_midi_parts(bytes: &[u8]) -> Result<Vec<MidiPartInfo>, ScoreError> {
    let smf = Smf::parse(bytes).map_err(|e| ScoreError::Midi(e.to_string()))?;
    let mut parts = Vec::new();
    for (track_index, track) in smf.tracks.iter().enumerate() {
        let mut name: Option<String> = None;
        let mut note_count = 0usize;
        for event in track {
            match event.kind {
                TrackEventKind::Meta(midly::MetaMessage::TrackName(name_bytes))
                    if name.is_none() =>
                {
                    if let Ok(n) = std::str::from_utf8(name_bytes) {
                        let n = n.trim();
                        if !n.is_empty() {
                            name = Some(n.to_string());
                        }
                    }
                }
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn { vel, .. },
                } if channel.as_int() != PERCUSSION_CHANNEL && vel.as_int() > 0 => {
                    note_count += 1;
                }
                _ => {}
            }
        }
        if note_count > 0 {
            parts.push(MidiPartInfo {
                track_index,
                name: name.unwrap_or_else(|| format!("Track {}", track_index + 1)),
                note_count,
            });
        }
    }
    Ok(parts)
}

/// Parse raw MIDI bytes into a [`ScoreModel`], reading note events from every
/// track (percussion excluded). Multi-part files merge — use
/// [`list_midi_parts`] + [`parse_midi_bytes_track`] to import one part.
pub fn parse_midi_bytes(bytes: &[u8]) -> Result<ScoreModel, ScoreError> {
    parse_midi_bytes_track(bytes, None)
}

/// Like [`parse_midi_bytes`], but when `track_filter` is `Some(i)` only track
/// `i`'s NOTE events are read. Meta events (title, tempo, signatures) are
/// still read from ALL tracks — Format-1 files keep them on a conductor
/// track the filter would otherwise silence.
pub fn parse_midi_bytes_track(
    bytes: &[u8],
    track_filter: Option<usize>,
) -> Result<ScoreModel, ScoreError> {
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
    // see consistent units. `None` until the FIRST Tempo event: the header
    // tempo is the score's tempo — later events are ritardandos/rubato that
    // must not retroactively rewrite the whole piece (#337 S1; v1 has no
    // tempo map).
    // (abs_tick, quarter-BPM) of the EARLIEST tempo event by tick time —
    // not iteration order: a conductor track's mid-piece tempo must not
    // beat another track's tick-0 header tempo (review S6).
    let mut tempo_quarter_bpm: Option<(u64, f64)> = None;

    // Collect all note events with absolute tick positions from all tracks
    let mut raw_notes: Vec<RawNote> = Vec::new();

    // Out-of-range filter = a caller bug; refuse with the real reason, not
    // the misleading "no playable notes" (review S5).
    if let Some(want) = track_filter {
        if want >= smf.tracks.len() {
            return Err(ScoreError::Midi(format!(
                "track {want} doesn't exist in this file ({} tracks)",
                smf.tracks.len()
            )));
        }
    }

    for (track_index, track) in smf.tracks.iter().enumerate() {
        // The filter silences a track's NOTES and its NAME; tempo and
        // signatures still come from every track (the conductor track
        // carries them), but the score's title/instrument must describe
        // the part the player chose — importing "Bass" must not label the
        // library entry "Trumpet" (review MUST-FIX 1).
        let notes_wanted = track_filter.is_none_or(|want| want == track_index);
        let mut abs_tick: u64 = 0;
        // Track active note-on events: (channel, key, velocity, start_tick).
        // Channel is included so that the same MIDI key sounding on two
        // different channels does not cross-match in `close_note`.
        let mut active_notes: Vec<(u8, u8, u8, u64)> = Vec::new();

        for event in track {
            abs_tick += event.delta.as_int() as u64;

            match event.kind {
                TrackEventKind::Meta(meta) => {
                    let is_name = matches!(meta, midly::MetaMessage::TrackName(_));
                    if !is_name || notes_wanted {
                        apply_meta_message(
                            meta,
                            abs_tick,
                            &mut title,
                            &mut instrument,
                            &mut time_signature,
                            &mut key_signature,
                            &mut tempo_quarter_bpm,
                        );
                    }
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    // Percussion notes are kit-piece ids, not pitches; a
                    // filtered-out track contributes no notes at all.
                    if ch == PERCUSSION_CHANNEL || !notes_wanted {
                        continue;
                    }
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

    // No sounding notes anywhere (drums-only file, click track, wrong track
    // filter): refuse calmly instead of importing an empty "score" the player
    // can't practice (#337 S1 — the silent half-score is the product bug).
    if raw_notes.is_empty() {
        return Err(ScoreError::Midi(
            "no playable notes found in this MIDI file — it may be a drum, click, \
             or marker track; try a different file or part"
                .to_string(),
        ));
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
    // 120 quarter-BPM is MIDI's defined default when no Tempo event exists.
    let tempo_bpm = tempo_quarter_bpm.map_or(120.0, |(_, bpm)| bpm) * (beat_type / 4.0);

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
    abs_tick: u64,
    title: &mut String,
    instrument: &mut Option<String>,
    time_signature: &mut TimeSignature,
    key_signature: &mut KeySignature,
    tempo_quarter_bpm: &mut Option<(u64, f64)>,
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
            // Microseconds per quarter-note → quarter-note BPM. The EARLIEST
            // event (by tick, across tracks) wins: the header tempo is the
            // piece's tempo; later Tempo events (a closing ritardando is the
            // classic) must not rewrite it.
            let uspqn = t.as_int() as f64;
            if uspqn > 0.0 && tempo_quarter_bpm.is_none_or(|(tick, _)| abs_tick < tick) {
                *tempo_quarter_bpm = Some((abs_tick, 60_000_000.0 / uspqn));
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

    /// The VA kit's committed sample must always work with the part
    /// picker exactly as the playbook describes (ship-gap coverage).
#[test]
fn va_sample_band_midi_lists_trumpet_and_bass() {
    let bytes = include_bytes!("../../../../va-testing-kit/samples/sample-band-c-major.mid");
    let parts = list_midi_parts(bytes).expect("kit sample parses");
    let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Trumpet", "Bass"], "drums + conductor omitted");
    let model = parse_midi_bytes_track(bytes, Some(parts[0].track_index)).expect("trumpet imports");
    assert_eq!(model.title, "Trumpet");
    assert!(model.measures.len() >= 2);
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

        // #337 S1: an empty file is a calm refusal, never a silent empty
        // "score" the player can't practice.
        let err = parse_midi_bytes(&buf).unwrap_err();
        assert!(
            matches!(&err, ScoreError::Midi(m) if m.contains("no playable notes")),
            "empty track should refuse calmly, got: {err:?}"
        );
    }

    /// Build a Format-1 band file: conductor track (tempo/meta only),
    /// "Trumpet" (4 notes, ch0), "Drums" (4 hits, ch9 percussion),
    /// "Bass" (2 notes, ch1).
    fn build_band_midi() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes()); // format 1
        buf.extend_from_slice(&4u16.to_be_bytes()); // 4 tracks
        buf.extend_from_slice(&480u16.to_be_bytes());

        let mut push_track = |events: Vec<u8>| {
            buf.extend_from_slice(b"MTrk");
            buf.extend_from_slice(&(events.len() as u32).to_be_bytes());
            buf.extend_from_slice(&events);
        };

        // Track 0: conductor — tempo 120, 4/4, no notes.
        let mut t0 = Vec::new();
        write_meta_event(&mut t0, 0, 0x51, &[0x07, 0xA1, 0x20]);
        write_meta_event(&mut t0, 0, 0x58, &[4, 2, 24, 8]);
        write_meta_event(&mut t0, 0, 0x2F, &[]);
        push_track(t0);

        // Track 1: Trumpet — 4 quarter notes on channel 0.
        let mut t1 = Vec::new();
        write_meta_event(&mut t1, 0, 0x03, b"Trumpet");
        for &key in &[60u8, 62, 64, 65] {
            write_midi_event(&mut t1, 0, 0x90, key, 80);
            write_midi_event(&mut t1, 480, 0x80, key, 0);
        }
        write_meta_event(&mut t1, 0, 0x2F, &[]);
        push_track(t1);

        // Track 2: Drums — 4 hits on channel 9 (percussion).
        let mut t2 = Vec::new();
        write_meta_event(&mut t2, 0, 0x03, b"Drums");
        for _ in 0..4 {
            write_midi_event(&mut t2, 0, 0x99, 36, 100); // ch9 note-on
            write_midi_event(&mut t2, 480, 0x89, 36, 0); // ch9 note-off
        }
        write_meta_event(&mut t2, 0, 0x2F, &[]);
        push_track(t2);

        // Track 3: Bass — 2 half notes on channel 1.
        let mut t3 = Vec::new();
        write_meta_event(&mut t3, 0, 0x03, b"Bass");
        for &key in &[36u8, 43] {
            write_midi_event(&mut t3, 0, 0x91, key, 80);
            write_midi_event(&mut t3, 960, 0x81, key, 0);
        }
        write_meta_event(&mut t3, 0, 0x2F, &[]);
        push_track(t3);

        buf
    }

    /// #337 S1: percussion (channel 10) never reaches the score — drum-kit
    /// note numbers are kit pieces, not pitches. A whole-file parse of the
    /// band fixture contains the trumpet and bass notes, zero drum hits.
    #[test]
    fn percussion_channel_notes_are_skipped() {
        let model = parse_midi_bytes(&build_band_midi()).expect("parse band MIDI");
        let midis: Vec<u8> = model
            .measures
            .iter()
            .flat_map(|m| m.notes.iter().filter(|n| !n.is_rest).map(|n| n.midi_number))
            .collect();
        assert_eq!(midis.len(), 6, "4 trumpet + 2 bass, no drums: {midis:?}");
        assert!(
            !midis
                .iter()
                .any(|&m| m == 36 && midis.iter().filter(|&&x| x == 36).count() > 1),
            "the 4 drum hits on note 36 must not appear (bass has one 36): {midis:?}"
        );
    }

    /// #337 S1: a drums-only file has nothing to practice → calm refusal.
    #[test]
    fn a_drums_only_file_refuses_calmly() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&480u16.to_be_bytes());
        let mut track = Vec::new();
        for _ in 0..4 {
            write_midi_event(&mut track, 0, 0x99, 38, 100);
            write_midi_event(&mut track, 480, 0x89, 38, 0);
        }
        write_meta_event(&mut track, 0, 0x2F, &[]);
        buf.extend_from_slice(b"MTrk");
        buf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        buf.extend_from_slice(&track);

        let err = parse_midi_bytes(&buf).unwrap_err();
        assert!(
            matches!(&err, ScoreError::Midi(m) if m.contains("no playable notes")),
            "drums-only must refuse with the calm message, got: {err:?}"
        );
    }

    /// #337 S1: `list_midi_parts` is the import picker's source of truth —
    /// playable tracks only (conductor and drum tracks omitted), with names,
    /// note counts, and ORIGINAL track indices.
    #[test]
    fn list_midi_parts_names_playable_tracks_only() {
        let parts = list_midi_parts(&build_band_midi()).expect("list parts");
        let summary: Vec<(usize, &str, usize)> = parts
            .iter()
            .map(|p| (p.track_index, p.name.as_str(), p.note_count))
            .collect();
        assert_eq!(
            summary,
            vec![(1, "Trumpet", 4), (3, "Bass", 2)],
            "conductor (no notes) and Drums (percussion) must be omitted"
        );
    }

    /// #337 S1: importing one part reads only that track's notes while meta
    /// (tempo, signatures) still comes from the conductor track.
    #[test]
    fn track_filter_imports_one_part_with_conductor_meta() {
        let model = parse_midi_bytes_track(&build_band_midi(), Some(3)).expect("import Bass");
        let midis: Vec<u8> = model
            .measures
            .iter()
            .flat_map(|m| m.notes.iter().filter(|n| !n.is_rest).map(|n| n.midi_number))
            .collect();
        assert_eq!(midis, vec![36, 43], "only the Bass track's notes");
        assert!(
            (model.tempo_bpm - 120.0).abs() < 0.1,
            "conductor-track tempo still applies to a filtered import"
        );
        // Review MUST-FIX 1: the imported part is NAMED after the chosen
        // track — picking Bass must not produce a score titled "Trumpet"
        // with a percussion "instrument".
        assert_eq!(model.title, "Bass", "the chosen track names the score");
        assert_eq!(model.instrument, None, "other tracks' names don't leak");
    }

    /// Review S5: an out-of-range track index is a caller bug and gets the
    /// real reason — not the misleading "no playable notes" message.
    #[test]
    fn out_of_range_track_filter_names_the_real_problem() {
        let err = parse_midi_bytes_track(&build_band_midi(), Some(99)).unwrap_err();
        assert!(
            matches!(&err, ScoreError::Midi(m) if m.contains("track 99 doesn't exist")),
            "got: {err:?}"
        );
    }

    /// Review S6: "first tempo" means first in TICK TIME across tracks —
    /// track order must not let a mid-piece conductor tempo beat another
    /// track's tick-0 header tempo.
    #[test]
    fn earliest_tempo_by_tick_wins_across_tracks() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&480u16.to_be_bytes());
        // Track 0: tempo 60 BPM but at tick 1920 (mid-piece).
        let mut t0 = Vec::new();
        write_variable_length(&mut t0, 1920);
        t0.push(0xFF);
        t0.push(0x51);
        write_variable_length(&mut t0, 3);
        t0.extend_from_slice(&[0x0F, 0x42, 0x40]); // 60 BPM
        write_meta_event(&mut t0, 0, 0x2F, &[]);
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(t0.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&t0);
        // Track 1: tempo 100 BPM at tick 0, plus a note.
        let mut t1 = Vec::new();
        write_meta_event(&mut t1, 0, 0x51, &[0x09, 0x27, 0xC0]); // 100 BPM
        write_midi_event(&mut t1, 0, 0x90, 60, 80);
        write_midi_event(&mut t1, 480, 0x80, 60, 0);
        write_meta_event(&mut t1, 0, 0x2F, &[]);
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(t1.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&t1);

        let model = parse_midi_bytes(&bytes).expect("parse");
        assert!(
            (model.tempo_bpm - 100.0).abs() < 0.5,
            "the tick-0 tempo wins regardless of track order, got {}",
            model.tempo_bpm
        );
    }

    /// Review MUST-FIX 2 (pickup coverage): a pickup bar — first note off
    /// the measure start — lands in measure 1 at its real beat with a
    /// leading rest, and measure numbering stays 1-based and contiguous.
    #[test]
    fn a_pickup_bar_keeps_sane_measures() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&480u16.to_be_bytes());
        let mut track = Vec::new();
        write_meta_event(&mut track, 0, 0x58, &[4, 2, 24, 8]); // 4/4
                                                               // Pickup: first note enters on beat 4 of measure 1 (tick 1440),
                                                               // then two on-beat notes carry into measure 2.
        write_variable_length(&mut track, 1440);
        track.push(0x90);
        track.push(67);
        track.push(80);
        write_midi_event(&mut track, 480, 0x80, 67, 0);
        for &key in &[60u8, 62] {
            write_midi_event(&mut track, 0, 0x90, key, 80);
            write_midi_event(&mut track, 480, 0x80, key, 0);
        }
        write_meta_event(&mut track, 0, 0x2F, &[]);
        buf.extend_from_slice(b"MTrk");
        buf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        buf.extend_from_slice(&track);

        let model = parse_midi_bytes(&buf).expect("parse pickup");
        let numbers: Vec<usize> = model.measures.iter().map(|m| m.number).collect();
        assert_eq!(numbers, vec![1, 2], "1-based contiguous measures");
        let m1 = &model.measures[0];
        let pickup = m1.notes.iter().find(|n| !n.is_rest).expect("pickup note");
        assert!(
            (pickup.start_beat - 3.0).abs() < 0.01,
            "pickup enters on beat 4 (0-based 3.0), got {}",
            pickup.start_beat
        );
        let lead_rest: f64 = m1
            .notes
            .iter()
            .take_while(|n| n.is_rest)
            .map(|n| n.duration_beats)
            .sum();
        assert!(
            (lead_rest - 3.0).abs() < 0.01,
            "the empty front of the pickup bar is rests, got {lead_rest}"
        );
    }

    /// #337 S1: the FIRST tempo event is the piece's tempo — a closing
    /// ritardando must not rewrite the whole score. Fails if last-wins
    /// returns.
    #[test]
    fn first_tempo_wins_over_a_closing_ritardando() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"MThd");
        buf.extend_from_slice(&6u32.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&480u16.to_be_bytes());
        let mut track = Vec::new();
        write_meta_event(&mut track, 0, 0x51, &[0x07, 0xA1, 0x20]); // 120 BPM
        write_midi_event(&mut track, 0, 0x90, 60, 80);
        write_midi_event(&mut track, 480, 0x80, 60, 0);
        // Ritardando: 60 BPM (1_000_000 us/qn = 0x0F4240) near the end.
        write_meta_event(&mut track, 0, 0x51, &[0x0F, 0x42, 0x40]);
        write_midi_event(&mut track, 0, 0x90, 62, 80);
        write_midi_event(&mut track, 480, 0x80, 62, 0);
        write_meta_event(&mut track, 0, 0x2F, &[]);
        buf.extend_from_slice(b"MTrk");
        buf.extend_from_slice(&(track.len() as u32).to_be_bytes());
        buf.extend_from_slice(&track);

        let model = parse_midi_bytes(&buf).expect("parse");
        assert!(
            (model.tempo_bpm - 120.0).abs() < 0.1,
            "first tempo (120) must win, got {}",
            model.tempo_bpm
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
