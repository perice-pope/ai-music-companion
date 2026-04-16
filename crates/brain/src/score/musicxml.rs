//! MusicXML parser — reads `.musicxml` / `.xml` files into a [`ScoreModel`].
//!
//! Uses `roxmltree` for lightweight, read-only XML parsing (no unsafe code).

use roxmltree::{Document, Node};

use super::{
    midi_to_hz, pitch_to_midi, Dynamic, KeyMode, KeySignature, Measure, ScoreError, ScoreModel,
    ScoreNote, TimeSignature,
};

/// Parse a MusicXML string into a [`ScoreModel`].
pub fn parse_musicxml_str(xml: &str) -> Result<ScoreModel, ScoreError> {
    let doc = Document::parse(xml).map_err(|e| ScoreError::MusicXml(e.to_string()))?;
    let root = doc.root_element();

    let title = extract_title(&root);
    let composer = extract_composer(&root);
    let instrument = extract_instrument(&root);

    let mut time_signature = TimeSignature::default();
    let mut key_signature = KeySignature::default();
    let mut tempo_bpm: f64 = 120.0;
    let mut divisions: f64 = 1.0;
    let mut measures = Vec::new();
    let mut current_dynamic: Option<Dynamic> = None;

    // Find the first <part> element
    let part = find_descendant(&root, "part");
    let part = match part {
        Some(p) => p,
        None => {
            return Err(ScoreError::MusicXml(
                "no <part> element found".to_string(),
            ))
        }
    };

    for measure_node in part.children().filter(|n| n.has_tag_name("measure")) {
        let measure_number = measure_node
            .attribute("number")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(measures.len() + 1);

        let mut notes = Vec::new();
        let mut current_beat: f64 = 0.0;

        for child in measure_node.children() {
            if child.has_tag_name("attributes") {
                if let Some(div) = find_descendant_text(&child, "divisions") {
                    if let Ok(d) = div.parse::<f64>() {
                        divisions = d;
                    }
                }
                if let Some(time_node) = find_descendant(&child, "time") {
                    if let (Some(beats_str), Some(bt_str)) = (
                        find_descendant_text(&time_node, "beats"),
                        find_descendant_text(&time_node, "beat-type"),
                    ) {
                        if let (Ok(b), Ok(bt)) = (beats_str.parse::<u8>(), bt_str.parse::<u8>()) {
                            time_signature = TimeSignature {
                                beats: b,
                                beat_type: bt,
                            };
                        }
                    }
                }
                if let Some(key_node) = find_descendant(&child, "key") {
                    if let Some(fifths_str) = find_descendant_text(&key_node, "fifths") {
                        if let Ok(f) = fifths_str.parse::<i8>() {
                            let mode = find_descendant_text(&key_node, "mode")
                                .map(|m| {
                                    if m.to_lowercase() == "minor" {
                                        KeyMode::Minor
                                    } else {
                                        KeyMode::Major
                                    }
                                })
                                .unwrap_or(KeyMode::Major);
                            key_signature = KeySignature { fifths: f, mode };
                        }
                    }
                }
            }

            if child.has_tag_name("direction") {
                // Extract tempo from <direction><sound tempo="...">
                if let Some(sound) = find_descendant(&child, "sound") {
                    if let Some(tempo_str) = sound.attribute("tempo") {
                        if let Ok(t) = tempo_str.parse::<f64>() {
                            tempo_bpm = t;
                        }
                    }
                }
                // Extract dynamics
                if let Some(dynamics_node) = find_descendant(&child, "dynamics") {
                    current_dynamic = parse_dynamic(&dynamics_node);
                }
            }

            if child.has_tag_name("note") {
                let is_rest = find_descendant(&child, "rest").is_some();

                let duration_divs = find_descendant_text(&child, "duration")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let duration_beats = if divisions > 0.0 {
                    duration_divs / divisions
                } else {
                    duration_divs
                };

                // Handle chord: chord notes share the same start_beat as the
                // previous note (don't advance current_beat).
                let is_chord = find_descendant(&child, "chord").is_some();
                let start_beat = if is_chord {
                    // Back up to where the previous note started.
                    (current_beat - duration_beats).max(0.0)
                } else {
                    current_beat
                };

                let (pitch_hz, midi_number) = if is_rest {
                    (0.0, 0)
                } else if let Some(pitch_node) = find_descendant(&child, "pitch") {
                    let step = find_descendant_text(&pitch_node, "step").unwrap_or_default();
                    let octave = find_descendant_text(&pitch_node, "octave")
                        .and_then(|s| s.parse::<i8>().ok())
                        .unwrap_or(4);
                    let alter = find_descendant_text(&pitch_node, "alter")
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    let midi = pitch_to_midi(&step, octave, alter);
                    let hz = midi_to_hz(midi);
                    (hz, midi.round() as u8)
                } else {
                    (0.0, 0)
                };

                notes.push(ScoreNote {
                    pitch_hz,
                    midi_number,
                    duration_beats,
                    start_beat,
                    dynamic: current_dynamic,
                    is_rest,
                });

                if !is_chord {
                    current_beat += duration_beats;
                }
            }

            // <forward> advances the beat cursor
            if child.has_tag_name("forward") {
                let dur = find_descendant_text(&child, "duration")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                current_beat += if divisions > 0.0 {
                    dur / divisions
                } else {
                    dur
                };
            }

            // <backup> moves the beat cursor backwards
            if child.has_tag_name("backup") {
                let dur = find_descendant_text(&child, "duration")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let backup_beats = if divisions > 0.0 {
                    dur / divisions
                } else {
                    dur
                };
                current_beat = (current_beat - backup_beats).max(0.0);
            }
        }

        measures.push(Measure {
            number: measure_number,
            notes,
        });
    }

    Ok(ScoreModel {
        title,
        composer,
        instrument,
        time_signature,
        key_signature,
        tempo_bpm,
        measures,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────

/// Extract the title from `<work><work-title>` or `<movement-title>`.
fn extract_title(root: &Node) -> String {
    // Try <work><work-title>
    if let Some(work) = find_descendant(root, "work") {
        if let Some(t) = find_descendant_text(&work, "work-title") {
            if !t.is_empty() {
                return t;
            }
        }
    }
    // Fallback: <movement-title>
    find_descendant_text(root, "movement-title").unwrap_or_else(|| "Untitled".to_string())
}

/// Extract the composer from `<identification><creator type="composer">`.
fn extract_composer(root: &Node) -> Option<String> {
    let identification = find_descendant(root, "identification")?;
    for child in identification.children() {
        if child.has_tag_name("creator") && child.attribute("type") == Some("composer") {
            return child.text().map(|t| t.trim().to_string());
        }
    }
    None
}

/// Extract the instrument name from the first `<part-name>`.
fn extract_instrument(root: &Node) -> Option<String> {
    let part_list = find_descendant(root, "part-list")?;
    let part_name = find_descendant_text(&part_list, "part-name")?;
    if part_name.is_empty() {
        None
    } else {
        Some(part_name)
    }
}

/// Parse a `<dynamics>` element into a [`Dynamic`].
fn parse_dynamic(dynamics_node: &Node) -> Option<Dynamic> {
    for child in dynamics_node.children() {
        if !child.is_element() {
            continue;
        }
        return match child.tag_name().name() {
            "ppp" => Some(Dynamic::PPP),
            "pp" => Some(Dynamic::PP),
            "p" => Some(Dynamic::P),
            "mp" => Some(Dynamic::MP),
            "mf" => Some(Dynamic::MF),
            "f" => Some(Dynamic::F),
            "ff" => Some(Dynamic::FF),
            "fff" => Some(Dynamic::FFF),
            _ => None,
        };
    }
    None
}

/// Find the first descendant element with the given tag name (depth-first).
fn find_descendant<'a>(node: &'a Node, tag: &str) -> Option<Node<'a, 'a>> {
    for child in node.children() {
        if child.has_tag_name(tag) {
            return Some(child);
        }
        if let Some(found) = find_descendant(&child, tag) {
            return Some(found);
        }
    }
    None
}

/// Find the first descendant with the given tag and return its trimmed text.
fn find_descendant_text(node: &Node, tag: &str) -> Option<String> {
    let found = find_descendant(node, tag)?;
    found.text().map(|t| t.trim().to_string())
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid MusicXML for a C major scale (C4-D4-E4-F4-G4-A4-B4-C5).
    const SIMPLE_SCALE_XML: &str = include_str!("../../tests/fixtures/simple_scale.musicxml");

    #[test]
    fn parse_simple_scale_musicxml() {
        let model = parse_musicxml_str(SIMPLE_SCALE_XML).expect("parse fixture");

        // 2 measures, 4 notes each = 8 notes total
        let total_notes: usize = model.measures.iter().map(|m| m.notes.len()).sum();
        assert_eq!(total_notes, 8, "Expected 8 notes, got {total_notes}");

        // Time signature 4/4
        assert_eq!(model.time_signature.beats, 4);
        assert_eq!(model.time_signature.beat_type, 4);

        // Tempo 120 BPM
        assert!((model.tempo_bpm - 120.0).abs() < 0.1);

        // Key of C major (0 sharps/flats)
        assert_eq!(model.key_signature.fifths, 0);
        assert_eq!(model.key_signature.mode, KeyMode::Major);

        // First note should be C4 (~261.63 Hz, MIDI 60)
        let first_note = &model.measures[0].notes[0];
        assert!(!first_note.is_rest);
        assert_eq!(first_note.midi_number, 60);
        assert!(
            (first_note.pitch_hz - 261.63).abs() < 0.1,
            "First note should be C4 (~261.63 Hz), got {}",
            first_note.pitch_hz
        );

        // Last note should be C5 (~523.25 Hz, MIDI 72)
        let last_measure = model.measures.last().unwrap();
        let last_note = last_measure.notes.last().unwrap();
        assert_eq!(last_note.midi_number, 72);
        assert!(
            (last_note.pitch_hz - 523.25).abs() < 0.1,
            "Last note should be C5 (~523.25 Hz), got {}",
            last_note.pitch_hz
        );
    }

    #[test]
    fn parse_extracts_title_and_composer() {
        let model = parse_musicxml_str(SIMPLE_SCALE_XML).expect("parse fixture");
        assert_eq!(model.title, "C Major Scale");
        assert_eq!(model.composer.as_deref(), Some("Test Composer"));
    }

    #[test]
    fn parse_handles_rests() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 3.1 Partwise//EN"
  "http://www.musicxml.org/dtds/partwise.dtd">
<score-partwise version="3.1">
  <part-list>
    <score-part id="P1"><part-name>Piano</part-name></score-part>
  </part-list>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>1</divisions>
        <time><beats>4</beats><beat-type>4</beat-type></time>
      </attributes>
      <note>
        <pitch><step>C</step><octave>4</octave></pitch>
        <duration>1</duration>
      </note>
      <note>
        <rest/>
        <duration>1</duration>
      </note>
      <note>
        <pitch><step>E</step><octave>4</octave></pitch>
        <duration>1</duration>
      </note>
      <note>
        <rest/>
        <duration>1</duration>
      </note>
    </measure>
  </part>
</score-partwise>"#;

        let model = parse_musicxml_str(xml).expect("parse rest XML");
        assert_eq!(model.measures.len(), 1);

        let notes = &model.measures[0].notes;
        assert_eq!(notes.len(), 4);
        assert!(!notes[0].is_rest, "First note should not be a rest");
        assert!(notes[1].is_rest, "Second note should be a rest");
        assert!(!notes[2].is_rest, "Third note should not be a rest");
        assert!(notes[3].is_rest, "Fourth note should be a rest");

        // Rests should have 0 Hz / MIDI 0
        assert_eq!(notes[1].pitch_hz, 0.0);
        assert_eq!(notes[1].midi_number, 0);
    }

    #[test]
    fn parse_rejects_malformed_xml() {
        let bad_xml = "<this is not valid xml>>>>>>";
        let result = parse_musicxml_str(bad_xml);
        assert!(result.is_err(), "Malformed XML should return an error");
        assert!(
            matches!(result.unwrap_err(), ScoreError::MusicXml(_)),
            "Error should be MusicXml variant"
        );
    }

    #[test]
    fn parse_rejects_xml_without_part() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <part-list>
    <score-part id="P1"><part-name>Piano</part-name></score-part>
  </part-list>
</score-partwise>"#;
        let result = parse_musicxml_str(xml);
        assert!(
            result.is_err(),
            "XML without <part> should return an error"
        );
    }

    #[test]
    fn parse_extracts_dynamics() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 3.1 Partwise//EN"
  "http://www.musicxml.org/dtds/partwise.dtd">
<score-partwise version="3.1">
  <part-list>
    <score-part id="P1"><part-name>Piano</part-name></score-part>
  </part-list>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>1</divisions>
      </attributes>
      <direction>
        <direction-type>
          <dynamics><f/></dynamics>
        </direction-type>
      </direction>
      <note>
        <pitch><step>C</step><octave>4</octave></pitch>
        <duration>1</duration>
      </note>
    </measure>
  </part>
</score-partwise>"#;

        let model = parse_musicxml_str(xml).expect("parse dynamics XML");
        let note = &model.measures[0].notes[0];
        assert_eq!(
            note.dynamic,
            Some(crate::score::Dynamic::F),
            "Note should have forte dynamic"
        );
    }

    #[test]
    fn note_start_beats_advance_correctly() {
        let model = parse_musicxml_str(SIMPLE_SCALE_XML).expect("parse fixture");
        let notes = &model.measures[0].notes;

        // Each note is 1 beat (quarter note), so start_beat should be 0, 1, 2, 3
        for (i, note) in notes.iter().enumerate() {
            assert!(
                (note.start_beat - i as f64).abs() < 0.001,
                "Note {i} start_beat should be {i}.0, got {}",
                note.start_beat
            );
        }
    }
}
