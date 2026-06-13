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
//! dynamics, rests). Enharmonic spelling is chosen with sharps; since the
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
    let part_name = model.instrument.as_deref().unwrap_or("Music");
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
            out.push_str("      </attributes>\n");

            out.push_str("      <direction placement=\"above\">\n");
            out.push_str(&format!(
                "        <sound tempo=\"{}\"/>\n",
                fmt_f64(model.tempo_bpm)
            ));
            out.push_str("      </direction>\n");
        }

        // Two onsets within this many beats count as simultaneous (a chord).
        const CHORD_ONSET_EPSILON: f64 = 1e-6;
        // Onset of the previous sounding note, so a note sharing it can be
        // flagged as a chord member. MusicXML stacks notes only when the
        // 2nd+ of a simultaneity carries a <chord/> element.
        let mut prev_sounding_start: Option<f64> = None;
        for note in &measure.notes {
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
            write_note(&mut out, note, is_chord);
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

/// Write a single `<note>` element (rest or pitched).
fn write_note(out: &mut String, note: &ScoreNote, is_chord: bool) {
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
        let (step, alter, octave) = midi_to_pitch(note.midi_number);
        out.push_str("        <pitch>\n");
        out.push_str(&format!("          <step>{step}</step>\n"));
        if alter != 0 {
            out.push_str(&format!("          <alter>{alter}</alter>\n"));
        }
        out.push_str(&format!("          <octave>{octave}</octave>\n"));
        out.push_str("        </pitch>\n");
    }
    out.push_str(&format!("        <duration>{duration_divs}</duration>\n"));
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

/// Map a MIDI note number to a MusicXML `(step, alter, octave)` triple,
/// spelling accidentals as sharps.
///
/// Inverse of the parser's `pitch_to_midi` for natural/sharp spellings:
/// `12 * (octave + 1) + semitone`. Round-trips by MIDI number regardless of
/// enharmonic choice.
fn midi_to_pitch(midi: u8) -> (char, i8, i8) {
    // (step letter, alter) for each pitch class, sharp spelling.
    const PITCH_CLASSES: [(char, i8); 12] = [
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
    let pc = (midi % 12) as usize;
    let octave = (midi / 12) as i8 - 1; // MIDI 60 = C4
    let (step, alter) = PITCH_CLASSES[pc];
    (step, alter, octave)
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
}
