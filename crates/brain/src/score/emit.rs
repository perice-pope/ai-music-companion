//! MusicXML emitter — serialises a [`ScoreModel`] back into MusicXML text.
//!
//! This is the inverse of [`crate::score::musicxml`]'s parser and the
//! canonical terminus of the Phase 2 import pipeline: every source (MIDI,
//! audio→MIDI, YouTube, OMR) is normalised into a [`ScoreModel`] and then
//! stored as MusicXML so the rest of the stack — OSMD rendering, the score
//! follower — has a single format to speak.
//!
//! Fidelity contract: the output is designed to round-trip through
//! [`crate::score::musicxml::parse_musicxml_str`] back into an equivalent
//! `ScoreModel` (pitch by MIDI number, durations in beats, time/key/tempo,
//! dynamics, rests). Enharmonic spelling follows the key signature (flats in
//! flat keys, sharps otherwise); since the
//! model carries pitch as a MIDI number, the exact spelling doesn't affect
//! round-trip fidelity.

use super::{Dynamic, KeyMode, ScoreModel, ScoreNote};

/// Ticks per quarter note used in the emitted `<divisions>`.
///
/// 480 is the common MIDI PPQ value; it divides cleanly by 2, 3, 4, 6, 8,
/// 12, 16, … so eighths, triplets, sixteenths and dotted values all land on
/// whole tick counts, keeping `duration_beats → duration_divs` exact for the
/// note values we expect from imports.
const DIVISIONS: u32 = 480;

/// Serialise a [`ScoreModel`] into a MusicXML 3.1 partwise document.
///
/// The output is a complete, parser-round-trippable document with a single
/// `<part>` (id `P1`). Intended for storing imported scores canonically.
pub fn score_model_to_musicxml(model: &ScoreModel) -> String {
    let mut out = String::with_capacity(1024 + model.measures.len() * 256);

    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    out.push_str(
        r#"<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 3.1 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">"#,
    );
    out.push('\n');
    out.push_str(r#"<score-partwise version="3.1">"#);
    out.push('\n');

    // ── Work title + identification (composer) ──
    out.push_str("  <work>\n");
    out.push_str(&format!(
        "    <work-title>{}</work-title>\n",
        escape_xml(&model.title)
    ));
    out.push_str("  </work>\n");

    if let Some(composer) = &model.composer {
        out.push_str("  <identification>\n");
        out.push_str(&format!(
            "    <creator type=\"composer\">{}</creator>\n",
            escape_xml(composer)
        ));
        out.push_str("  </identification>\n");
    }

    // ── Part list ──
    // #356: the staff label. A filtered MIDI import carries the chosen part's
    // name as the TITLE (instrument stays None so other tracks' names can't
    // leak), so the title is the honest second choice — "Music" would show
    // OSMD's anonymous default over a part the player picked by name.
    let part_name = model
        .instrument
        .as_deref()
        .or(Some(model.title.as_str()))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Music");
    out.push_str("  <part-list>\n");
    out.push_str("    <score-part id=\"P1\">\n");
    out.push_str(&format!(
        "      <part-name>{}</part-name>\n",
        escape_xml(part_name)
    ));
    out.push_str("    </score-part>\n");
    out.push_str("  </part-list>\n");

    // ── The single part ──
    out.push_str("  <part id=\"P1\">\n");

    // Dynamic state tracked across notes/measures so we only emit a
    // `<direction><dynamics>` when the marking *changes* — matching how the
    // parser carries `current_dynamic` forward.
    let mut current_dynamic: Option<Dynamic> = None;

    for (idx, measure) in model.measures.iter().enumerate() {
        out.push_str(&format!("    <measure number=\"{}\">\n", measure.number));

        // Attributes (divisions/key/time) + tempo only on the first measure;
        // the parser carries them forward, and re-emitting would be noise.
        if idx == 0 {
            out.push_str("      <attributes>\n");
            out.push_str(&format!("        <divisions>{DIVISIONS}</divisions>\n"));
            out.push_str("        <key>\n");
            out.push_str(&format!(
                "          <fifths>{}</fifths>\n",
                model.key_signature.fifths
            ));
            out.push_str(&format!(
                "          <mode>{}</mode>\n",
                match model.key_signature.mode {
                    KeyMode::Major => "major",
                    KeyMode::Minor => "minor",
                }
            ));
            out.push_str("        </key>\n");
            out.push_str("        <time>\n");
            out.push_str(&format!(
                "          <beats>{}</beats>\n",
                model.time_signature.beats
            ));
            out.push_str(&format!(
                "          <beat-type>{}</beat-type>\n",
                model.time_signature.beat_type
            ));
            out.push_str("        </time>\n");
            // #417-3: keyboard scores are a GRAND STAFF — two staves, treble
            // over bass, both clefs declared up front (an empty bass staff
            // with whole rests is how real piano music shows a silent left
            // hand). XSD order: <staves> before <clef>.
            if model.grand_staff {
                out.push_str("        <staves>2</staves>\n");
                out.push_str("        <clef number=\"1\">\n");
                out.push_str("          <sign>G</sign>\n");
                out.push_str("          <line>2</line>\n");
                out.push_str("        </clef>\n");
                out.push_str("        <clef number=\"2\">\n");
                out.push_str("          <sign>F</sign>\n");
                out.push_str("          <line>4</line>\n");
                out.push_str("        </clef>\n");
            }
            out.push_str("      </attributes>\n");

            // #356: a <direction> with no <direction-type> child is invalid
            // MusicXML, and OSMD 1.9.x reacts by silently dropping EVERY note
            // of the measure that contains it — imports rendered with a blank
            // first measure ("half the notes" on a two-measure score). The
            // metronome mark makes the element valid (and shows the tempo);
            // <sound tempo> stays because the parser round-trips from it.
            out.push_str("      <direction placement=\"above\">\n");
            out.push_str("        <direction-type>\n");
            out.push_str("          <metronome>\n");
            out.push_str(&format!(
                "            <beat-unit>{}</beat-unit>\n",
                beat_unit_name(model.time_signature.beat_type)
            ));
            out.push_str(&format!(
                "            <per-minute>{}</per-minute>\n",
                fmt_f64(model.tempo_bpm)
            ));
            out.push_str("          </metronome>\n");
            out.push_str("        </direction-type>\n");
            out.push_str(&format!(
                "        <sound tempo=\"{}\"/>\n",
                fmt_f64(model.tempo_bpm)
            ));
            out.push_str("      </direction>\n");
        }

        // Two onsets within this many beats count as simultaneous (a chord).
        const CHORD_ONSET_EPSILON: f64 = 1e-6;
        // #417-3: on a grand staff every note (and rest) carries its staff.
        let staves = model.grand_staff.then(|| staff_positions(&measure.notes));
        let beams = beam_positions(
            &measure.notes,
            model.time_signature.beats,
            model.time_signature.beat_type,
            staves.as_deref(),
        );
        // Onset of the previous sounding note, so a note sharing it can be
        // flagged as a chord member. MusicXML stacks notes only when the
        // 2nd+ of a simultaneity carries a <chord/> element.
        let mut prev_sounding_start: Option<f64> = None;
        for (note_idx, note) in measure.notes.iter().enumerate() {
            // Emit a dynamics direction when the marking changes (skip on
            // rests — dynamics attach to sounding notes).
            if !note.is_rest && note.dynamic != current_dynamic {
                if let Some(dynamic) = note.dynamic {
                    out.push_str("      <direction placement=\"below\">\n");
                    out.push_str("        <direction-type>\n");
                    out.push_str(&format!(
                        "          <dynamics><{tag}/></dynamics>\n",
                        tag = dynamic_tag(dynamic)
                    ));
                    out.push_str("        </direction-type>\n");
                    out.push_str("      </direction>\n");
                    current_dynamic = Some(dynamic);
                }
            }

            let is_chord = !note.is_rest
                && prev_sounding_start
                    .is_some_and(|prev| (note.start_beat - prev).abs() < CHORD_ONSET_EPSILON);
            write_note(
                &mut out,
                note,
                is_chord,
                model.key_signature.fifths < 0,
                beams[note_idx],
                staves.as_ref().map(|s| s[note_idx]),
            );
            if !note.is_rest {
                prev_sounding_start = Some(note.start_beat);
            }
        }

        out.push_str("    </measure>\n");
    }

    out.push_str("  </part>\n");
    out.push_str("</score-partwise>\n");
    out
}

/// A note's place in a `<beam number="1">` group.
#[derive(Clone, Copy, PartialEq, Debug)]
enum BeamPos {
    Begin,
    Continue,
    End,
}

impl BeamPos {
    fn tag(self) -> &'static str {
        match self {
            BeamPos::Begin => "begin",
            BeamPos::Continue => "continue",
            BeamPos::End => "end",
        }
    }
}

/// Close the open beam group: groups of 2+ get begin/continue…/end; a lone
/// eighth keeps its flag (no beam element at all).
fn close_beam_group(group: &mut Vec<usize>, out: &mut [Option<BeamPos>]) {
    if group.len() >= 2 {
        out[group[0]] = Some(BeamPos::Begin);
        for &i in &group[1..group.len() - 1] {
            out[i] = Some(BeamPos::Continue);
        }
        out[*group.last().unwrap()] = Some(BeamPos::End);
    }
    group.clear();
}

/// Beam assignment for one measure's notes: consecutive eighth notes beam in
/// groups of 2–4 that never cross the metric region boundary — the
/// HALF-MEASURE in even `X/4` meters (beat 1 in 2/4, beat 2 in 4/4, beat 3
/// in 6/4; a full beat of two eighths beams as a pair, a 4/4 half-measure
/// run beams as a four), per-beat in odd meters like 3/4.
///
/// Rests, longer values, and gaps break a group; a lone eighth keeps its
/// flag. Chord members (same onset as the previous sounding note) carry no
/// beam of their own — they ride their anchor. Non-quarter denominators
/// (6/8 …) are left unbeamed for now: compound-meter grouping is a
/// different rule and flags are honest there.
///
/// #417-3: on a grand staff (`staves` present) a staff change also breaks
/// the group — a run crossing middle C must not beam across staves
/// (cross-staff beaming is real notation, but renderer support is shaky
/// and flags are honest).
fn beam_positions(
    notes: &[ScoreNote],
    beats: u8,
    beat_type: u8,
    staves: Option<&[u8]>,
) -> Vec<Option<BeamPos>> {
    const EPS: f64 = 1e-6;
    let mut out = vec![None; notes.len()];
    if beat_type != 4 {
        return out;
    }
    let region_beats = if beats.is_multiple_of(2) {
        f64::from(beats) / 2.0
    } else {
        1.0
    };
    // Region index of a start position. The +EPS absorbs parser-accumulated
    // float drift: six triplet eighths sum to 1.999999…8, which must still
    // classify as the region STARTING at 2.0, not the one before it.
    let region_of = |start: f64| ((start + EPS) / region_beats).floor();

    // Indices of the group currently being built + where it ends in time.
    let mut group: Vec<usize> = Vec::new();
    let mut group_end = 0.0_f64;
    let mut prev_sounding_start: Option<f64> = None;

    for (i, n) in notes.iter().enumerate() {
        let chord_member =
            !n.is_rest && prev_sounding_start.is_some_and(|prev| (n.start_beat - prev).abs() < EPS);
        if !n.is_rest {
            prev_sounding_start = Some(n.start_beat);
        }
        if chord_member {
            continue;
        }
        let is_eighth = !n.is_rest && (n.duration_beats - 0.5).abs() < EPS;
        if !is_eighth {
            close_beam_group(&mut group, &mut out);
            continue;
        }
        let contiguous = (n.start_beat - group_end).abs() < EPS;
        let same_region = group
            .first()
            .is_none_or(|&first| region_of(notes[first].start_beat) == region_of(n.start_beat));
        let same_staff = group
            .first()
            .is_none_or(|&first| staves.is_none_or(|s| s[first] == s[i]));
        let extends_group = contiguous && same_region && same_staff && group.len() < 4;
        if !group.is_empty() && !extends_group {
            close_beam_group(&mut group, &mut out);
        }
        group.push(i);
        group_end = n.start_beat + n.duration_beats;
    }
    close_beam_group(&mut group, &mut out);
    out
}

/// #417-3: per-note staff assignment for a grand staff. Staff 1 (treble)
/// at/above middle C (midi 60), staff 2 (bass) below. A chord — notes
/// sharing an onset — stays WHOLE on its lowest note's staff; cross-staff
/// chords need `<voice>`/`<backup>` writing and are a spec non-goal. A rest
/// follows the previous sounding note's staff, so a bass phrase's breaths
/// stay in the left hand (opening rests read treble).
fn staff_positions(notes: &[ScoreNote]) -> Vec<u8> {
    const EPS: f64 = 1e-6;
    const MIDDLE_C: u8 = 60;
    let mut out = vec![1u8; notes.len()];
    let mut current = 1u8;
    let mut i = 0;
    while i < notes.len() {
        if notes[i].is_rest {
            out[i] = current;
            i += 1;
            continue;
        }
        // The chord group sharing this onset (members are adjacent by
        // construction — see the main loop's `prev_sounding_start` rule).
        let mut end = i + 1;
        let mut lowest = notes[i].midi_number;
        while end < notes.len()
            && !notes[end].is_rest
            && (notes[end].start_beat - notes[i].start_beat).abs() < EPS
        {
            lowest = lowest.min(notes[end].midi_number);
            end += 1;
        }
        current = if lowest >= MIDDLE_C { 1 } else { 2 };
        for slot in &mut out[i..end] {
            *slot = current;
        }
        i = end;
    }
    out
}

/// Write a single `<note>` element (rest or pitched).
fn write_note(
    out: &mut String,
    note: &ScoreNote,
    is_chord: bool,
    flats: bool,
    beam: Option<BeamPos>,
    staff: Option<u8>,
) {
    let duration_divs = beats_to_divs(note.duration_beats);

    out.push_str("      <note>\n");
    // A <chord/> on the 2nd+ simultaneous note stacks it on the previous note
    // instead of advancing time. The first note of a chord never carries it.
    if is_chord {
        out.push_str("        <chord/>\n");
    }
    if note.is_rest {
        out.push_str("        <rest/>\n");
    } else {
        let (step, alter, octave) = midi_to_pitch(note.midi_number, flats);
        out.push_str("        <pitch>\n");
        out.push_str(&format!("          <step>{step}</step>\n"));
        if alter != 0 {
            out.push_str(&format!("          <alter>{alter}</alter>\n"));
        }
        out.push_str(&format!("          <octave>{octave}</octave>\n"));
        out.push_str("        </pitch>\n");
    }
    out.push_str(&format!("        <duration>{duration_divs}</duration>\n"));
    // Beamed notes are eighths by construction (see `beam_positions`). The
    // <type> makes the beam well-formed MusicXML — renderers pair the beam
    // with the note value rather than inferring it from the duration.
    // XSD child order within <note>: <type> … <staff> … <beam>.
    if beam.is_some() {
        out.push_str("        <type>eighth</type>\n");
    }
    if let Some(staff) = staff {
        out.push_str(&format!("        <staff>{staff}</staff>\n"));
    }
    if let Some(pos) = beam {
        out.push_str(&format!(
            "        <beam number=\"1\">{}</beam>\n",
            pos.tag()
        ));
    }
    out.push_str("      </note>\n");
}

/// Convert a duration in beats (quarter-note units) to divisions ticks.
///
/// `DIVISIONS` is ticks-per-quarter, and a beat in the model *is* a quarter
/// (see the parser: `duration_beats = duration_divs / divisions`). Rounded
/// to the nearest tick; clamped to ≥ 0.
fn beats_to_divs(beats: f64) -> u32 {
    (beats * DIVISIONS as f64).round().max(0.0) as u32
}

/// Map a MIDI note number to a MusicXML `(step, alter, octave)` triple —
/// flat spellings when `flats` (flat key signatures), sharps otherwise.
///
/// Inverse of the parser's `pitch_to_midi` for natural/sharp spellings:
/// `12 * (octave + 1) + semitone`. Round-trips by MIDI number regardless of
/// enharmonic choice.
pub(crate) fn midi_to_pitch(midi: u8, flats: bool) -> (char, i8, i8) {
    // (step letter, alter) for each pitch class, sharp spelling.
    const SHARP_CLASSES: [(char, i8); 12] = [
        ('C', 0), // 0
        ('C', 1), // 1  C#
        ('D', 0), // 2
        ('D', 1), // 3  D#
        ('E', 0), // 4
        ('F', 0), // 5
        ('F', 1), // 6  F#
        ('G', 0), // 7
        ('G', 1), // 8  G#
        ('A', 0), // 9
        ('A', 1), // 10 A#
        ('B', 0), // 11
    ];
    // Flat spelling for flat key signatures (fifths < 0): Bb-major material
    // reads as Bb, not A# (#277 follow-up — a wall of sharps in a flat key is
    // unreadable to a student).
    const FLAT_CLASSES: [(char, i8); 12] = [
        ('C', 0),  // 0
        ('D', -1), // 1  Db
        ('D', 0),  // 2
        ('E', -1), // 3  Eb
        ('E', 0),  // 4
        ('F', 0),  // 5
        ('G', -1), // 6  Gb
        ('G', 0),  // 7
        ('A', -1), // 8  Ab
        ('A', 0),  // 9
        ('B', -1), // 10 Bb
        ('B', 0),  // 11
    ];
    let pc = (midi % 12) as usize;
    let octave = (midi / 12) as i8 - 1; // MIDI 60 = C4
    let (step, alter) = if flats {
        FLAT_CLASSES[pc]
    } else {
        SHARP_CLASSES[pc]
    };
    (step, alter, octave)
}

/// MusicXML `<beat-unit>` note-value name for a time signature's beat type.
///
/// The model's `tempo_bpm` is in signature-beat units (see the MIDI parser),
/// so the metronome mark pairs that number with the signature's beat unit.
fn beat_unit_name(beat_type: u8) -> &'static str {
    match beat_type {
        1 => "whole",
        2 => "half",
        8 => "eighth",
        16 => "16th",
        32 => "32nd",
        _ => "quarter",
    }
}

/// MusicXML element tag for a [`Dynamic`].
fn dynamic_tag(dynamic: Dynamic) -> &'static str {
    match dynamic {
        Dynamic::PPP => "ppp",
        Dynamic::PP => "pp",
        Dynamic::P => "p",
        Dynamic::MP => "mp",
        Dynamic::MF => "mf",
        Dynamic::F => "f",
        Dynamic::FF => "ff",
        Dynamic::FFF => "fff",
    }
}

/// Format an `f64` for attribute output: integers print without a trailing
/// `.0` (so `120.0` → `"120"`), fractional values keep their decimals.
fn fmt_f64(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        // Trim to a sane precision and drop trailing zeros.
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Escape the five XML predefined entities for text/attribute content.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::musicxml::parse_musicxml_str;
    use crate::score::{KeySignature, Measure, TimeSignature};

    fn note(midi: u8, beats: f64, start: f64) -> ScoreNote {
        ScoreNote {
            pitch_hz: crate::score::midi_to_hz(midi as f64),
            midi_number: midi,
            duration_beats: beats,
            start_beat: start,
            dynamic: None,
            is_rest: false,
        }
    }

    fn rest(beats: f64, start: f64) -> ScoreNote {
        ScoreNote {
            pitch_hz: 0.0,
            midi_number: 0,
            duration_beats: beats,
            start_beat: start,
            dynamic: None,
            is_rest: true,
        }
    }

    fn c_major_scale() -> ScoreModel {
        // C4 D4 E4 F4 in measure 1, quarter notes.
        let notes = vec![
            note(60, 1.0, 0.0),
            note(62, 1.0, 1.0),
            note(64, 1.0, 2.0),
            note(65, 1.0, 3.0),
        ];
        ScoreModel {
            title: "Round Trip".to_string(),
            composer: Some("Tester".to_string()),
            instrument: Some("Trumpet".to_string()),
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            measures: vec![Measure { number: 1, notes }],
            grand_staff: false,
        }
    }

    #[test]
    fn output_parses_back() {
        let xml = score_model_to_musicxml(&c_major_scale());
        let reparsed = parse_musicxml_str(&xml).expect("emitted XML must parse");
        assert_eq!(reparsed.title, "Round Trip");
        assert_eq!(reparsed.composer.as_deref(), Some("Tester"));
        assert_eq!(reparsed.instrument.as_deref(), Some("Trumpet"));
        assert_eq!(reparsed.measures.len(), 1);
        assert_eq!(reparsed.measures[0].notes.len(), 4);
    }

    #[test]
    fn round_trip_preserves_pitches_and_durations() {
        let original = c_major_scale();
        let xml = score_model_to_musicxml(&original);
        let reparsed = parse_musicxml_str(&xml).unwrap();

        let orig_notes = &original.measures[0].notes;
        let rt_notes = &reparsed.measures[0].notes;
        assert_eq!(orig_notes.len(), rt_notes.len());
        for (o, r) in orig_notes.iter().zip(rt_notes) {
            assert_eq!(o.midi_number, r.midi_number, "MIDI number must round-trip");
            assert!(
                (o.duration_beats - r.duration_beats).abs() < 1e-9,
                "duration {} vs {}",
                o.duration_beats,
                r.duration_beats
            );
        }
    }

    #[test]
    fn round_trip_preserves_time_key_tempo() {
        let mut model = c_major_scale();
        model.time_signature = TimeSignature {
            beats: 3,
            beat_type: 4,
        };
        model.key_signature = KeySignature {
            fifths: 2,
            mode: KeyMode::Minor,
        };
        model.tempo_bpm = 96.0;

        let reparsed = parse_musicxml_str(&score_model_to_musicxml(&model)).unwrap();
        assert_eq!(reparsed.time_signature, model.time_signature);
        assert_eq!(reparsed.key_signature, model.key_signature);
        assert!((reparsed.tempo_bpm - 96.0).abs() < 1e-9);
    }

    #[test]
    fn simultaneous_notes_emit_chord_elements() {
        // A C-major triad (all three notes share onset 0.0) followed by a
        // single note. The 2nd and 3rd triad notes must carry <chord/>; the
        // root never does, and the trailing note (distinct onset) must not.
        let notes = vec![
            note(60, 1.0, 0.0), // C4  — chord root, no <chord/>
            note(64, 1.0, 0.0), // E4  — <chord/>
            note(67, 1.0, 0.0), // G4  — <chord/>
            note(72, 1.0, 1.0), // C5  — separate onset, no <chord/>
        ];
        let model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        let xml = score_model_to_musicxml(&model);
        assert_eq!(
            xml.matches("<chord/>").count(),
            2,
            "exactly the 2nd and 3rd triad notes carry <chord/>:\n{xml}"
        );
    }

    #[test]
    fn sequential_notes_emit_no_chord_elements() {
        // Four distinct onsets — nothing should be flagged as a chord.
        let xml = score_model_to_musicxml(&c_major_scale());
        assert!(
            !xml.contains("<chord/>"),
            "no chord tags for sequential notes:\n{xml}"
        );
    }

    #[test]
    fn sharps_round_trip_by_midi_number() {
        // C#4 (61), F#4 (66), A#4 (70) — accidentals via <alter>.
        let model = ScoreModel {
            title: "Sharps".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 100.0,
            grand_staff: false,
            measures: vec![Measure {
                number: 1,
                notes: vec![note(61, 1.0, 0.0), note(66, 1.0, 1.0), note(70, 1.0, 2.0)],
            }],
        };
        let reparsed = parse_musicxml_str(&score_model_to_musicxml(&model)).unwrap();
        let midis: Vec<u8> = reparsed.measures[0]
            .notes
            .iter()
            .map(|n| n.midi_number)
            .collect();
        assert_eq!(midis, vec![61, 66, 70]);
    }

    #[test]
    fn rests_round_trip() {
        let model = ScoreModel {
            title: "With Rest".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            grand_staff: false,
            measures: vec![Measure {
                number: 1,
                notes: vec![note(60, 1.0, 0.0), rest(1.0, 1.0), note(62, 2.0, 2.0)],
            }],
        };
        let reparsed = parse_musicxml_str(&score_model_to_musicxml(&model)).unwrap();
        let notes = &reparsed.measures[0].notes;
        assert_eq!(notes.len(), 3);
        assert!(!notes[0].is_rest);
        assert!(notes[1].is_rest, "middle note must round-trip as a rest");
        assert!(!notes[2].is_rest);
    }

    #[test]
    fn dynamics_round_trip() {
        let mut n0 = note(60, 1.0, 0.0);
        n0.dynamic = Some(Dynamic::F);
        let mut n1 = note(62, 1.0, 1.0);
        n1.dynamic = Some(Dynamic::F); // unchanged — should not re-emit but stays F
        let mut n2 = note(64, 1.0, 2.0);
        n2.dynamic = Some(Dynamic::PP); // change → new direction
        let model = ScoreModel {
            title: "Dyn".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            grand_staff: false,
            measures: vec![Measure {
                number: 1,
                notes: vec![n0, n1, n2],
            }],
        };
        let reparsed = parse_musicxml_str(&score_model_to_musicxml(&model)).unwrap();
        let notes = &reparsed.measures[0].notes;
        assert_eq!(notes[0].dynamic, Some(Dynamic::F));
        assert_eq!(notes[1].dynamic, Some(Dynamic::F), "F carries forward");
        assert_eq!(notes[2].dynamic, Some(Dynamic::PP));
    }

    #[test]
    fn fractional_durations_round_trip() {
        // Eighth (0.5) and triplet-eighth (1/3) — exact at DIVISIONS=480.
        let model = ScoreModel {
            title: "Frac".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            grand_staff: false,
            measures: vec![Measure {
                number: 1,
                notes: vec![note(60, 0.5, 0.0), note(62, 1.0 / 3.0, 0.5)],
            }],
        };
        let reparsed = parse_musicxml_str(&score_model_to_musicxml(&model)).unwrap();
        let notes = &reparsed.measures[0].notes;
        assert!((notes[0].duration_beats - 0.5).abs() < 1e-6);
        assert!((notes[1].duration_beats - 1.0 / 3.0).abs() < 1e-3);
    }

    #[test]
    fn special_characters_in_title_are_escaped() {
        let mut model = c_major_scale();
        model.title = "Jazz & \"Blues\" <Etude>".to_string();
        let xml = score_model_to_musicxml(&model);
        assert!(xml.contains("&amp;"), "ampersand must be escaped");
        assert!(xml.contains("&lt;") && xml.contains("&gt;"));
        let reparsed = parse_musicxml_str(&xml).unwrap();
        assert_eq!(reparsed.title, "Jazz & \"Blues\" <Etude>");
    }

    #[test]
    fn multi_measure_round_trip() {
        let model = ScoreModel {
            title: "Two Bars".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            grand_staff: false,
            measures: vec![
                Measure {
                    number: 1,
                    notes: vec![note(60, 4.0, 0.0)],
                },
                Measure {
                    number: 2,
                    notes: vec![note(67, 4.0, 0.0)],
                },
            ],
        };
        let reparsed = parse_musicxml_str(&score_model_to_musicxml(&model)).unwrap();
        assert_eq!(reparsed.measures.len(), 2);
        assert_eq!(reparsed.measures[0].notes[0].midi_number, 60);
        assert_eq!(reparsed.measures[1].notes[0].midi_number, 67);
        // Time/key/tempo declared only in measure 1 still apply throughout.
        assert_eq!(reparsed.tempo_bpm, 120.0);
    }

    /// #356: every emitted `<direction>` must carry a `<direction-type>`
    /// child. A bare `<direction><sound/></direction>` is invalid MusicXML
    /// and OSMD 1.9.x silently drops every note of the measure containing
    /// it — the "half the notes render" VA finding. Fails if the tempo
    /// direction loses its metronome wrapper.
    #[test]
    fn every_direction_carries_a_direction_type() {
        let mut model = c_major_scale();
        model.measures[0].notes[0].dynamic = Some(Dynamic::MF);
        let xml = score_model_to_musicxml(&model);
        let directions = xml.matches("<direction ").count() + xml.matches("<direction>").count();
        let direction_types = xml.matches("<direction-type>").count();
        assert!(directions >= 2, "tempo + dynamics directions expected");
        assert_eq!(
            directions, direction_types,
            "every <direction> needs a <direction-type> child (OSMD drops the \
             measure's notes otherwise):\n{xml}"
        );
    }

    /// #356: the tempo direction is a real metronome mark — beat unit from
    /// the time signature, per-minute from the model — and `<sound tempo>`
    /// still round-trips. In 6/8 the model's tempo is eighth-BPM, so the
    /// mark must pair it with "eighth", not "quarter".
    #[test]
    fn tempo_metronome_speaks_the_signature_beat_unit() {
        let mut model = c_major_scale();
        model.time_signature = TimeSignature {
            beats: 6,
            beat_type: 8,
        };
        model.tempo_bpm = 240.0;
        let xml = score_model_to_musicxml(&model);
        assert!(
            xml.contains("<beat-unit>eighth</beat-unit>"),
            "6/8 tempo pairs with the eighth beat unit:\n{xml}"
        );
        assert!(xml.contains("<per-minute>240</per-minute>"));
        let reparsed = parse_musicxml_str(&xml).unwrap();
        assert!(
            (reparsed.tempo_bpm - 240.0).abs() < 1e-9,
            "tempo still round-trips via <sound tempo>"
        );
    }

    /// Cut time (2/2) is a realistic MIDI import (`denom_pow=1`); its tempo
    /// mark must pair with the half-note beat unit. Fails if the beat-unit
    /// table's non-default rows regress.
    #[test]
    fn cut_time_tempo_pairs_with_the_half_note() {
        let mut model = c_major_scale();
        model.time_signature = TimeSignature {
            beats: 2,
            beat_type: 2,
        };
        model.tempo_bpm = 60.0;
        let xml = score_model_to_musicxml(&model);
        assert!(
            xml.contains("<beat-unit>half</beat-unit>"),
            "2/2 tempo pairs with the half beat unit:\n{xml}"
        );
        assert!(xml.contains("<per-minute>60</per-minute>"));
        // And the finer units map, so a 16th-based signature never shows a
        // quarter mark.
        assert_eq!(beat_unit_name(16), "16th");
        assert_eq!(beat_unit_name(32), "32nd");
        assert_eq!(beat_unit_name(1), "whole");
    }

    /// #356: a filtered band import carries the part's name as the TITLE
    /// (instrument None) — the staff must be labeled with it, never OSMD's
    /// anonymous "Music".
    #[test]
    fn part_name_falls_back_to_title_when_instrument_missing() {
        let mut model = c_major_scale();
        model.instrument = None;
        model.title = "Trumpet".to_string();
        let xml = score_model_to_musicxml(&model);
        assert!(
            xml.contains("<part-name>Trumpet</part-name>"),
            "title labels the part when instrument is absent:\n{xml}"
        );
    }

    /// The instrument, when present, still wins over the title.
    #[test]
    fn part_name_prefers_instrument_over_title() {
        let model = c_major_scale(); // instrument: Trumpet, title: Round Trip
        let xml = score_model_to_musicxml(&model);
        assert!(xml.contains("<part-name>Trumpet</part-name>"));
        assert!(!xml.contains("<part-name>Round Trip</part-name>"));
    }

    /// With neither instrument nor a usable title, the label stays "Music"
    /// rather than an empty element.
    #[test]
    fn part_name_defaults_to_music_when_nothing_usable() {
        let mut model = c_major_scale();
        model.instrument = None;
        model.title = "   ".to_string();
        let xml = score_model_to_musicxml(&model);
        assert!(xml.contains("<part-name>Music</part-name>"));
    }

    /// A title used as the part label goes through XML escaping like any
    /// other text content.
    #[test]
    fn part_name_from_title_is_escaped() {
        let mut model = c_major_scale();
        model.instrument = None;
        model.title = "Horn & Flugel".to_string();
        let xml = score_model_to_musicxml(&model);
        assert!(xml.contains("<part-name>Horn &amp; Flugel</part-name>"));
    }

    /// Eight straight eighths in 4/4 beam as two groups of four, split at
    /// the half-measure — never one eight-note beam, never lone flags.
    /// Fails if grouping, the size cap, or the region boundary regresses.
    #[test]
    fn straight_eighths_beam_in_fours_per_half_measure() {
        let notes: Vec<ScoreNote> = (0..8)
            .map(|i| note(60 + i as u8, 0.5, i as f64 * 0.5))
            .collect();
        let model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        let xml = score_model_to_musicxml(&model);
        assert_eq!(xml.matches("<beam number=\"1\">begin</beam>").count(), 2);
        assert_eq!(xml.matches("<beam number=\"1\">continue</beam>").count(), 4);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 2);
        // The split lands exactly at beat 2: note 4 (start 2.0) begins the
        // second group, so the sequence is begin,cont,cont,end ×2.
        let positions: Vec<&str> = xml
            .match_indices("<beam number=\"1\">")
            .map(|(i, _)| &xml[i + 17..i + 20])
            .collect();
        assert_eq!(
            positions,
            vec!["beg", "con", "con", "end", "beg", "con", "con", "end"]
        );
    }

    /// A rest inside the run breaks the beam: eighth,eighth,rest,eighth →
    /// a pair (begin/end) and a lone flagged eighth with no beam element.
    #[test]
    fn rests_break_beams_and_singletons_stay_flagged() {
        let notes = vec![
            note(60, 0.5, 0.0),
            note(62, 0.5, 0.5),
            rest(0.5, 1.0),
            note(64, 0.5, 1.5),
        ];
        let model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        let xml = score_model_to_musicxml(&model);
        assert_eq!(xml.matches("<beam number=\"1\">begin</beam>").count(), 1);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 1);
        assert_eq!(
            xml.matches("<beam").count(),
            2,
            "the isolated eighth after the rest keeps its flag:\n{xml}"
        );
    }

    /// Two contiguous eighths straddling the half-measure (starts 1.5 and
    /// 2.0) must NOT beam across the boundary — each is a singleton, so no
    /// beam at all. Fails if the region check drops out of the grouping.
    #[test]
    fn beams_never_cross_the_half_measure_boundary() {
        let notes = vec![
            note(60, 1.5, 0.0),
            note(62, 0.5, 1.5),
            note(64, 0.5, 2.0),
            note(65, 1.5, 2.5),
        ];
        let model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        let xml = score_model_to_musicxml(&model);
        assert!(
            !xml.contains("<beam"),
            "eighths on either side of beat 2 must not share a beam:\n{xml}"
        );
    }

    /// Quarter notes never beam — the plain scale emits no beam elements
    /// (and no <type>, which the emitter only writes alongside a beam).
    #[test]
    fn quarter_notes_emit_no_beams() {
        let xml = score_model_to_musicxml(&c_major_scale());
        assert!(!xml.contains("<beam"), "no beams on quarters:\n{xml}");
        assert!(!xml.contains("<type>"));
    }

    /// In 3/4 the region is one beat, so six straight eighths beam as three
    /// pairs — begin/end three times, no continue.
    #[test]
    fn three_four_beams_in_pairs_per_beat() {
        let notes: Vec<ScoreNote> = (0..6)
            .map(|i| note(60 + i as u8, 0.5, i as f64 * 0.5))
            .collect();
        let mut model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        model.time_signature = TimeSignature {
            beats: 3,
            beat_type: 4,
        };
        let xml = score_model_to_musicxml(&model);
        assert_eq!(xml.matches("<beam number=\"1\">begin</beam>").count(), 3);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 3);
        assert_eq!(xml.matches("<beam number=\"1\">continue</beam>").count(), 0);
    }

    /// Review M2: in 2/4 the half-measure is beat 1 — four straight eighths
    /// beam as TWO PAIRS, never one four across the bar's midpoint. Fails
    /// if the region reverts to a hard-coded 2 beats.
    #[test]
    fn two_four_beams_in_pairs_across_its_half_measure() {
        let notes: Vec<ScoreNote> = (0..4)
            .map(|i| note(60 + i as u8, 0.5, i as f64 * 0.5))
            .collect();
        let mut model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        model.time_signature = TimeSignature {
            beats: 2,
            beat_type: 4,
        };
        let xml = score_model_to_musicxml(&model);
        assert_eq!(xml.matches("<beam number=\"1\">begin</beam>").count(), 2);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 2);
        assert_eq!(xml.matches("<beam number=\"1\">continue</beam>").count(), 0);
    }

    /// Review M2: in 6/4 the half-measure is beat 3 — two contiguous
    /// eighths at starts 2.5 and 3.0 straddle it and must not beam.
    #[test]
    fn six_four_beams_never_cross_beat_three() {
        let notes = vec![
            note(60, 2.5, 0.0),
            note(62, 0.5, 2.5),
            note(64, 0.5, 3.0),
            note(65, 2.5, 3.5),
        ];
        let mut model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        model.time_signature = TimeSignature {
            beats: 6,
            beat_type: 4,
        };
        let xml = score_model_to_musicxml(&model);
        assert!(
            !xml.contains("<beam"),
            "eighths on either side of beat 3 in 6/4 must not beam:\n{xml}"
        );
    }

    /// Review M4: parser-accumulated float drift must not shift the region.
    /// Six triplet eighths sum to 1.999999…8; the straight eighths that
    /// follow (drifted starts ≈2.0, 2.5, 3.0, 3.5) still all classify into
    /// the second half-measure and beam as one four — no orphaned flag.
    #[test]
    fn drifted_starts_still_group_with_their_half_measure() {
        let mut notes: Vec<ScoreNote> = Vec::new();
        let mut cursor = 0.0_f64;
        for i in 0..6 {
            notes.push(note(60 + i, 1.0 / 3.0, cursor));
            cursor += 1.0 / 3.0; // accumulates like the MusicXML parser
        }
        assert_ne!(cursor, 2.0, "the fixture must actually exercise drift");
        for i in 0..4 {
            notes.push(note(67 + i, 0.5, cursor));
            cursor += 0.5;
        }
        let model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        let xml = score_model_to_musicxml(&model);
        assert_eq!(
            xml.matches("<beam number=\"1\">begin</beam>").count(),
            1,
            "the four straight eighths beam as ONE group:\n{xml}"
        );
        assert_eq!(xml.matches("<beam number=\"1\">continue</beam>").count(), 2);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 1);
    }

    /// Compound meter (6/8) is deliberately left unbeamed — its grouping
    /// rule (threes per dotted quarter) isn't implemented, and flags are
    /// honest where a wrong beam would lie about the meter.
    #[test]
    fn compound_meter_stays_unbeamed() {
        let notes: Vec<ScoreNote> = (0..6)
            .map(|i| note(60 + i as u8, 0.5, i as f64 * 0.5))
            .collect();
        let mut model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        model.time_signature = TimeSignature {
            beats: 6,
            beat_type: 8,
        };
        let xml = score_model_to_musicxml(&model);
        assert!(!xml.contains("<beam"), "6/8 must stay flag-only:\n{xml}");
    }

    /// Beamed eighth-note block chords: the chord ANCHORS beam; the stacked
    /// members (<chord/> notes) carry no beam of their own — they ride the
    /// anchor's stem. Fails if members join the group (which would make the
    /// group overshoot its cap and renderers double-beam the stack).
    #[test]
    fn chord_members_ride_the_anchor_beam() {
        // Two eighth-note dyads: anchors at 0.0 and 0.5, each with one
        // stacked member sharing the onset.
        let notes = vec![
            note(60, 0.5, 0.0),
            note(64, 0.5, 0.0), // member
            note(62, 0.5, 0.5),
            note(65, 0.5, 0.5), // member
        ];
        let model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        let xml = score_model_to_musicxml(&model);
        assert_eq!(xml.matches("<chord/>").count(), 2);
        assert_eq!(
            xml.matches("<beam").count(),
            2,
            "exactly the two anchors beam (begin + end):\n{xml}"
        );
        assert_eq!(xml.matches("<beam number=\"1\">begin</beam>").count(), 1);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 1);
        // PLACEMENT, not just counts (test-audit mutant: shifting the beam
        // onto a member leaves every count identical): a <note> carrying
        // <chord/> must never also carry <beam>, and both anchors — E4/F4
        // members, C4/D4 anchors — must be the beamed ones.
        for block in xml.split("<note>").skip(1) {
            let block = block.split("</note>").next().unwrap();
            assert!(
                !(block.contains("<chord/>") && block.contains("<beam")),
                "a chord member must never carry its own beam:\n{block}"
            );
            if block.contains("<beam") {
                assert!(
                    block.contains("<step>C</step>") || block.contains("<step>D</step>"),
                    "beams belong on the anchors (C4, D4):\n{block}"
                );
            }
        }
    }

    /// An onset GAP with no rest note between two eighths (legal in the
    /// model) also breaks the beam: eighths at 0.0 and 1.0 are not
    /// contiguous, so each is a singleton and nothing beams. Fails if the
    /// contiguity condition drops out of the grouping (test-audit mutant 7
    /// — every other "gap" test realizes the gap as an explicit rest).
    #[test]
    fn onset_gaps_without_rests_break_beams() {
        let notes = vec![note(60, 0.5, 0.0), note(62, 0.5, 1.0)];
        let model = ScoreModel {
            measures: vec![Measure { number: 1, notes }],
            ..c_major_scale()
        };
        let xml = score_model_to_musicxml(&model);
        assert!(
            !xml.contains("<beam"),
            "non-contiguous eighths in the same half-measure must not beam:\n{xml}"
        );
    }

    /// Beamed output still round-trips: the parser ignores <type>/<beam>
    /// and reads back the same pitches and durations.
    #[test]
    fn beamed_output_round_trips() {
        let notes: Vec<ScoreNote> = (0..8)
            .map(|i| note(60 + i as u8, 0.5, i as f64 * 0.5))
            .collect();
        let model = ScoreModel {
            measures: vec![Measure {
                number: 1,
                notes: notes.clone(),
            }],
            ..c_major_scale()
        };
        let reparsed = parse_musicxml_str(&score_model_to_musicxml(&model)).unwrap();
        let rt = &reparsed.measures[0].notes;
        assert_eq!(rt.len(), 8);
        for (o, r) in notes.iter().zip(rt) {
            assert_eq!(o.midi_number, r.midi_number);
            assert!((o.duration_beats - r.duration_beats).abs() < 1e-9);
        }
    }

    /// #277: under a flat key signature the emitter spells FLATS — MIDI 70 in
    /// Bb major (fifths -2) is <step>B</step><alter>-1</alter>, never A#. And
    /// the flat spelling roundtrips through the parser to the same MIDI note.
    /// Fails if the fifths<0 condition is inverted or a flat-table entry is
    /// corrupted.
    #[test]
    fn flat_keys_spell_flats_and_roundtrip() {
        let model = ScoreModel {
            title: "Bb drill".to_owned(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature {
                beats: 4,
                beat_type: 4,
            },
            key_signature: KeySignature {
                fifths: -2,
                mode: KeyMode::Major,
            },
            tempo_bpm: 80.0,
            grand_staff: false,
            measures: vec![Measure {
                number: 1,
                notes: vec![
                    note(70, 2.0, 0.0), // Bb4
                    note(63, 2.0, 2.0), // Eb4
                ],
            }],
        };
        let xml = score_model_to_musicxml(&model);
        assert!(
            xml.contains("<step>B</step>\n          <alter>-1</alter>"),
            "MIDI 70 under fifths -2 must spell Bb, got:\n{xml}"
        );
        assert!(
            xml.contains("<step>E</step>\n          <alter>-1</alter>"),
            "MIDI 63 under fifths -2 must spell Eb"
        );
        assert!(xml.contains("<fifths>-2</fifths>"));
        assert!(!xml.contains("<alter>1</alter>"), "no sharps in a flat key");

        // Roundtrip: the parser reads alter=-1 back to the same MIDI numbers.
        let parsed = crate::score::musicxml::parse_musicxml_str(&xml).expect("parses");
        let midis: Vec<u8> = parsed.measures[0]
            .notes
            .iter()
            .map(|n| n.midi_number)
            .collect();
        assert_eq!(midis, vec![70, 63]);
    }

    // ── #417-3: grand staff for keyboard scores ──────────────────────────

    /// A one-measure grand-staff model over the given notes.
    fn grand(notes: Vec<ScoreNote>) -> ScoreModel {
        ScoreModel {
            title: "Piano Drill".to_string(),
            composer: None,
            instrument: Some("Piano".to_string()),
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 90.0,
            measures: vec![Measure { number: 1, notes }],
            grand_staff: true,
        }
    }

    /// Every `<staff>N</staff>` in emission order.
    fn staff_seq(xml: &str) -> Vec<u8> {
        xml.match_indices("<staff>")
            .map(|(i, _)| xml[i + 7..i + 8].parse().unwrap())
            .collect()
    }

    /// #417-3 AC1+AC2: two staves, both clefs, and per-note staff split at
    /// middle C — midi 60 itself is TREBLE (the pinned boundary).
    #[test]
    fn grand_staff_emits_two_staves_and_splits_at_middle_c() {
        let xml = score_model_to_musicxml(&grand(vec![
            note(48, 1.0, 0.0), // C3 → bass
            note(64, 1.0, 1.0), // E4 → treble
            note(60, 1.0, 2.0), // middle C exactly → treble
            note(59, 1.0, 3.0), // B3 → bass
        ]));
        assert!(
            xml.contains("<staves>2</staves>"),
            "staves declared:\n{xml}"
        );
        assert!(
            xml.contains("<clef number=\"1\">\n          <sign>G</sign>"),
            "treble clef on staff 1"
        );
        assert!(
            xml.contains("<clef number=\"2\">\n          <sign>F</sign>"),
            "bass clef on staff 2"
        );
        assert_eq!(staff_seq(&xml), vec![2, 1, 1, 2]);
    }

    /// #417-3 AC3: a chord straddling middle C stays WHOLE on its lowest
    /// note's staff — never split across staves (spec non-goal).
    #[test]
    fn a_chord_stays_whole_on_its_lowest_notes_staff() {
        let xml = score_model_to_musicxml(&grand(vec![
            note(48, 1.0, 0.0), // C3 ┐
            note(64, 1.0, 0.0), // E4 ├ one chord, lowest below middle C
            note(67, 1.0, 0.0), // G4 ┘
            note(72, 1.0, 1.0), // C5 alone → treble
        ]));
        assert_eq!(staff_seq(&xml), vec![2, 2, 2, 1]);
        // Still ONE chord: exactly two <chord/> members ride the anchor.
        assert_eq!(xml.matches("<chord/>").count(), 2);
    }

    /// #417-3 AC7: rests follow the previous sounding note's staff, so a
    /// bass phrase's breaths stay in the left hand; opening rests read
    /// treble (nothing has sounded yet).
    #[test]
    fn rests_follow_the_previous_notes_staff() {
        let xml = score_model_to_musicxml(&grand(vec![
            rest(1.0, 0.0),     // opening rest → treble by default
            note(50, 1.0, 1.0), // D3 → bass
            rest(1.0, 2.0),     // breath inside the bass phrase → bass
            note(52, 1.0, 3.0), // E3 → bass
        ]));
        assert_eq!(staff_seq(&xml), vec![1, 2, 2, 2]);
    }

    /// #417-3: an eighth run crossing middle C breaks its beam at the staff
    /// change — two pairs, never one four-group beamed across staves
    /// (renderer support for cross-staff beams is shaky; flags are honest).
    #[test]
    fn beams_break_at_the_staff_change() {
        let run = |midi: u8, start: f64| note(midi, 0.5, start);
        let xml = score_model_to_musicxml(&grand(vec![
            run(55, 0.0), // G3 bass ┐ pair
            run(57, 0.5), // A3 bass ┘
            run(60, 1.0), // C4 treble ┐ pair
            run(64, 1.5), // E4 treble ┘
        ]));
        assert_eq!(xml.matches("<beam number=\"1\">begin</beam>").count(), 2);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 2);
        assert_eq!(xml.matches("continue").count(), 0);
    }

    /// #417-3 AC4: a non-grand-staff model's output carries no staff
    /// machinery at all — byte-for-byte today's single-staff behavior.
    #[test]
    fn non_grand_staff_output_is_unchanged() {
        let mut model = grand(vec![note(48, 2.0, 0.0), note(64, 2.0, 2.0)]);
        model.grand_staff = false;
        let xml = score_model_to_musicxml(&model);
        assert!(!xml.contains("<staves>"));
        assert!(!xml.contains("<clef"));
        assert!(!xml.contains("<staff>"));
    }

    /// #417-3 parse edge: grand-staff XML round-trips through the parser —
    /// notes and durations survive, staff/clef elements don't break it.
    #[test]
    fn grand_staff_xml_parses_back() {
        let model = grand(vec![
            note(48, 1.0, 0.0),
            note(64, 1.0, 1.0),
            rest(1.0, 2.0),
            note(72, 1.0, 3.0),
        ]);
        let xml = score_model_to_musicxml(&model);
        let parsed =
            crate::score::musicxml::parse_musicxml_str(&xml).expect("grand-staff XML parses");
        let midis: Vec<u8> = parsed.measures[0]
            .notes
            .iter()
            .filter(|n| !n.is_rest)
            .map(|n| n.midi_number)
            .collect();
        assert_eq!(midis, vec![48, 64, 72]);
    }
}
