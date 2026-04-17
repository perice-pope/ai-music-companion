//! MusicXML parser — reads `.musicxml` / `.xml` files into a [`ScoreModel`].
//!
//! Uses `roxmltree` for lightweight, read-only XML parsing (no unsafe code).

use roxmltree::{Document, Node, ParsingOptions};

use super::{
    midi_to_hz, pitch_to_midi, Dynamic, KeyMode, KeySignature, Measure, ScoreError, ScoreModel,
    ScoreNote, TimeSignature,
};

/// Parse XML with DTD allowed. Real-world MusicXML files (and our fixtures)
/// start with a `<!DOCTYPE score-partwise PUBLIC ...>` declaration; the
/// default `Document::parse` rejects these as a security precaution.
/// Since we only parse user-provided local files (no XXE risk from
/// remote input), opt-in to DTD parsing.
fn parse_xml(xml: &str) -> Result<Document<'_>, ScoreError> {
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    Document::parse_with_options(xml, opts).map_err(|e| ScoreError::MusicXml(e.to_string()))
}

/// Parse a MusicXML string into a [`ScoreModel`], selecting the first `<part>`.
///
/// For multi-part scores, use [`parse_musicxml_str_part`] to choose a specific
/// part.
pub fn parse_musicxml_str(xml: &str) -> Result<ScoreModel, ScoreError> {
    parse_musicxml_str_part(xml, 0)
}

/// List part names (from `<part-list><score-part><part-name>`) in document
/// order. Names that are missing or empty are reported as "Part N" so the
/// returned vector length always matches the number of `<part>` elements.
pub fn list_parts(xml: &str) -> Result<Vec<String>, ScoreError> {
    let doc = parse_xml(xml)?;
    let root = doc.root_element();

    // Map from score-part id -> part-name text
    let mut id_to_name: Vec<(String, String)> = Vec::new();
    if let Some(part_list) = find_descendant(&root, "part-list") {
        for sp in part_list
            .children()
            .filter(|n| n.has_tag_name("score-part"))
        {
            let id = sp.attribute("id").unwrap_or("").to_string();
            let name = find_descendant_text(&sp, "part-name").unwrap_or_default();
            id_to_name.push((id, name));
        }
    }

    let parts: Vec<Node> = root
        .descendants()
        .filter(|n| n.has_tag_name("part"))
        .collect();

    let mut names = Vec::with_capacity(parts.len());
    for (idx, part) in parts.iter().enumerate() {
        let id = part.attribute("id").unwrap_or("");
        let name = id_to_name
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, n)| n.clone())
            .unwrap_or_default();
        if name.is_empty() {
            names.push(format!("Part {}", idx + 1));
        } else {
            names.push(name);
        }
    }

    Ok(names)
}

/// Parse a MusicXML string, selecting the part at `part_index` (zero-based,
/// in document order).
pub fn parse_musicxml_str_part(xml: &str, part_index: usize) -> Result<ScoreModel, ScoreError> {
    let doc = parse_xml(xml)?;
    let root = doc.root_element();

    let title = extract_title(&root);
    let composer = extract_composer(&root);

    let mut time_signature = TimeSignature::default();
    let mut key_signature = KeySignature::default();
    let mut tempo_bpm: f64 = 120.0;
    let mut divisions: f64 = 1.0;
    let mut measures = Vec::new();
    let mut current_dynamic: Option<Dynamic> = None;

    // Collect ALL <part> elements (not just the first) so the caller can
    // choose which part to practice in a multi-instrument score.
    let parts: Vec<Node> = root
        .descendants()
        .filter(|n| n.has_tag_name("part"))
        .collect();

    if parts.is_empty() {
        return Err(ScoreError::MusicXml("no <part> element found".to_string()));
    }

    let part = parts
        .get(part_index)
        .ok_or(ScoreError::PartNotFound(part_index))?;

    // Instrument name for the selected part (matches `id` attribute against
    // `<score-part id="...">` in `<part-list>`).
    let instrument = extract_instrument_for(&root, part.attribute("id"));

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
                    // A <pitch> child must provide a valid step and octave;
                    // fabricating defaults (e.g. C4) when the data is missing
                    // or malformed produces silently-wrong notes that are
                    // worse than an error.
                    let step_raw = find_descendant_text(&pitch_node, "step").ok_or_else(|| {
                        ScoreError::MusicXml(format!(
                            "missing <step> in pitch at measure {measure_number}"
                        ))
                    })?;
                    let step = step_raw.to_uppercase();
                    if !matches!(step.as_str(), "C" | "D" | "E" | "F" | "G" | "A" | "B") {
                        return Err(ScoreError::MusicXml(format!(
                            "invalid pitch step '{step_raw}' at measure {measure_number} \
                             (expected one of C, D, E, F, G, A, B)"
                        )));
                    }

                    let octave_str =
                        find_descendant_text(&pitch_node, "octave").ok_or_else(|| {
                            ScoreError::MusicXml(format!(
                                "missing <octave> in pitch at measure {measure_number}"
                            ))
                        })?;
                    let octave = octave_str.parse::<i8>().map_err(|_| {
                        ScoreError::MusicXml(format!(
                            "invalid octave '{octave_str}' at measure {measure_number}"
                        ))
                    })?;

                    // <alter> is optional; defaults to 0 (natural). An
                    // unparseable value IS an error (don't silently drop it).
                    let alter = match find_descendant_text(&pitch_node, "alter") {
                        None => 0.0,
                        Some(s) => s.parse::<f64>().map_err(|_| {
                            ScoreError::MusicXml(format!(
                                "invalid alter '{s}' at measure {measure_number}"
                            ))
                        })?,
                    };

                    let midi = pitch_to_midi(&step, octave, alter);
                    let hz = midi_to_hz(midi);
                    (hz, midi.round() as u8)
                } else {
                    return Err(ScoreError::MusicXml(format!(
                        "note at measure {measure_number} has neither <pitch> nor <rest>"
                    )));
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

/// Extract the instrument name for a specific `<part id="...">` by matching
/// against `<score-part id="...">` in `<part-list>`. Falls back to the first
/// `<part-name>` if the id is unknown.
fn extract_instrument_for(root: &Node, part_id: Option<&str>) -> Option<String> {
    let part_list = find_descendant(root, "part-list")?;
    if let Some(id) = part_id {
        for score_part in part_list
            .children()
            .filter(|n| n.has_tag_name("score-part"))
        {
            if score_part.attribute("id") == Some(id) {
                return find_descendant_text(&score_part, "part-name").filter(|s| !s.is_empty());
            }
        }
    }
    find_descendant_text(&part_list, "part-name").filter(|s| !s.is_empty())
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
///
/// Lifetime `'input` is the document's input-string lifetime; `'a` is the
/// arena lifetime. Threading both through explicitly (rather than tying the
/// returned node to the reference lifetime) is required so the recursive
/// call's return value doesn't borrow from the local `child`.
fn find_descendant<'a, 'input>(node: &Node<'a, 'input>, tag: &str) -> Option<Node<'a, 'input>> {
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
        assert!(result.is_err(), "XML without <part> should return an error");
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

    /// Two-part MusicXML: Trumpet plays C4 D4, Trombone plays G3 F3.
    const TWO_PART_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<score-partwise version="3.1">
  <part-list>
    <score-part id="P1"><part-name>Trumpet</part-name></score-part>
    <score-part id="P2"><part-name>Trombone</part-name></score-part>
  </part-list>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>1</divisions>
        <time><beats>4</beats><beat-type>4</beat-type></time>
      </attributes>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>
      <note><pitch><step>D</step><octave>4</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
  <part id="P2">
    <measure number="1">
      <attributes>
        <divisions>1</divisions>
        <time><beats>4</beats><beat-type>4</beat-type></time>
      </attributes>
      <note><pitch><step>G</step><octave>3</octave></pitch><duration>1</duration></note>
      <note><pitch><step>F</step><octave>3</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;

    #[test]
    fn parse_multi_part_selects_first_part_by_default() {
        let model = parse_musicxml_str(TWO_PART_XML).expect("parse two-part XML");
        let midi_numbers: Vec<u8> = model.measures[0]
            .notes
            .iter()
            .map(|n| n.midi_number)
            .collect();
        assert_eq!(
            midi_numbers,
            vec![60, 62],
            "Default (part 0) should be Trumpet (C4=60, D4=62)"
        );
        assert_eq!(model.instrument.as_deref(), Some("Trumpet"));
    }

    #[test]
    fn parse_multi_part_selects_second_part() {
        let model = parse_musicxml_str_part(TWO_PART_XML, 1).expect("parse part 1");
        let midi_numbers: Vec<u8> = model.measures[0]
            .notes
            .iter()
            .map(|n| n.midi_number)
            .collect();
        assert_eq!(
            midi_numbers,
            vec![55, 53],
            "Part 1 should be Trombone (G3=55, F3=53)"
        );
        assert_eq!(model.instrument.as_deref(), Some("Trombone"));
    }

    #[test]
    fn parse_multi_part_out_of_range_returns_part_not_found() {
        let err = parse_musicxml_str_part(TWO_PART_XML, 99).unwrap_err();
        assert!(
            matches!(err, ScoreError::PartNotFound(99)),
            "Expected PartNotFound(99), got: {err}"
        );
    }

    #[test]
    fn list_parts_returns_names_in_document_order() {
        let names = list_parts(TWO_PART_XML).expect("list parts");
        assert_eq!(names, vec!["Trumpet".to_string(), "Trombone".to_string()]);
    }

    #[test]
    fn list_parts_fills_missing_names_with_placeholders() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list>
    <score-part id="P1"><part-name></part-name></score-part>
    <score-part id="P2"><part-name>Violin</part-name></score-part>
  </part-list>
  <part id="P1"><measure number="1"/></part>
  <part id="P2"><measure number="1"/></part>
</score-partwise>"#;
        let names = list_parts(xml).expect("list parts");
        assert_eq!(names, vec!["Part 1".to_string(), "Violin".to_string()]);
    }

    #[test]
    fn pitch_missing_step_returns_error() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions></attributes>
      <note><pitch><octave>4</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let err = parse_musicxml_str(xml).unwrap_err();
        match err {
            ScoreError::MusicXml(msg) => {
                assert!(
                    msg.contains("step") && msg.contains("measure 1"),
                    "Error should mention missing step at measure 1, got: {msg}"
                );
            }
            other => panic!("Expected MusicXml error, got: {other}"),
        }
    }

    #[test]
    fn pitch_invalid_step_returns_error() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions></attributes>
      <note><pitch><step>Q</step><octave>4</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let err = parse_musicxml_str(xml).unwrap_err();
        assert!(
            matches!(err, ScoreError::MusicXml(ref m) if m.contains("'Q'")),
            "Expected error mentioning invalid step 'Q', got: {err}"
        );
    }

    #[test]
    fn pitch_invalid_octave_returns_error() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions></attributes>
      <note><pitch><step>C</step><octave>not-a-number</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let err = parse_musicxml_str(xml).unwrap_err();
        assert!(
            matches!(err, ScoreError::MusicXml(ref m) if m.contains("octave")),
            "Expected error mentioning invalid octave, got: {err}"
        );
    }

    #[test]
    fn note_without_pitch_or_rest_returns_error() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions></attributes>
      <note><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let err = parse_musicxml_str(xml).unwrap_err();
        assert!(
            matches!(err, ScoreError::MusicXml(ref m) if m.contains("neither")),
            "Expected error about note without pitch or rest, got: {err}"
        );
    }

    #[test]
    fn pitch_step_is_case_insensitive() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions></attributes>
      <note><pitch><step>c</step><octave>4</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let model = parse_musicxml_str(xml).expect("lowercase step should parse");
        assert_eq!(model.measures[0].notes[0].midi_number, 60);
    }
}
