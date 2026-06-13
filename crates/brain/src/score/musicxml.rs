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
        // Remember the most recent non-chord note's start_beat so chord notes
        // can share it. Using `current_beat - duration_beats` is only correct
        // when the chord leader's duration equals the chord note's duration,
        // which breaks for tied or mixed-value chords.
        let mut last_note_start: f64 = 0.0;

        for child in measure_node.children() {
            if child.has_tag_name("attributes") {
                apply_attributes(
                    &child,
                    measure_number,
                    &mut divisions,
                    &mut time_signature,
                    &mut key_signature,
                )?;
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
                let parsed = parse_note(
                    &child,
                    divisions,
                    current_dynamic,
                    current_beat,
                    last_note_start,
                    measure_number,
                )?;
                // A chord note shares the previous note's start beat and does
                // not advance the cursor; a normal note does both.
                if !parsed.is_chord {
                    last_note_start = parsed.note.start_beat;
                    current_beat += parsed.note.duration_beats;
                }
                notes.push(parsed.note);
            }

            // <forward> advances the beat cursor
            if child.has_tag_name("forward") {
                let dur = parse_required_duration(&child, "forward", measure_number)?;
                current_beat += dur / divisions;
            }

            // <backup> moves the beat cursor backwards
            if child.has_tag_name("backup") {
                let dur = parse_required_duration(&child, "backup", measure_number)?;
                let backup_beats = dur / divisions;
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

/// Apply an `<attributes>` element to the running parser state: divisions,
/// time signature, and key signature. Errors on an invalid `<divisions>`
/// (it's used as a divisor downstream, so a non-positive value is rejected
/// rather than silently coerced).
fn apply_attributes(
    attributes: &Node,
    measure_number: usize,
    divisions: &mut f64,
    time_signature: &mut TimeSignature,
    key_signature: &mut KeySignature,
) -> Result<(), ScoreError> {
    if let Some(div) = find_descendant_text(attributes, "divisions") {
        let d = div.parse::<f64>().map_err(|_| {
            ScoreError::MusicXml(format!(
                "invalid <divisions> value '{div}' at measure {measure_number}"
            ))
        })?;
        if !(d.is_finite() && d > 0.0) {
            return Err(ScoreError::MusicXml(format!(
                "<divisions> must be a positive finite number (got {d}) \
                 at measure {measure_number}"
            )));
        }
        *divisions = d;
    }
    if let Some(time_node) = find_descendant(attributes, "time") {
        if let (Some(beats_str), Some(bt_str)) = (
            find_descendant_text(&time_node, "beats"),
            find_descendant_text(&time_node, "beat-type"),
        ) {
            if let (Ok(b), Ok(bt)) = (beats_str.parse::<u8>(), bt_str.parse::<u8>()) {
                *time_signature = TimeSignature {
                    beats: b,
                    beat_type: bt,
                };
            }
        }
    }
    if let Some(key_node) = find_descendant(attributes, "key") {
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
                *key_signature = KeySignature { fifths: f, mode };
            }
        }
    }
    Ok(())
}

/// A parsed `<note>`: the score note plus whether it was a chord member. Chord
/// notes share the previous note's start beat and don't advance the cursor.
struct ParsedNote {
    note: ScoreNote,
    is_chord: bool,
}

/// Parse a `<note>` element at the given beat cursor.
///
/// `<duration>` is required except on grace notes (`<grace/>`), which legally
/// omit it and are treated as zero-beat. A rest yields a 0 Hz / 0 MIDI note;
/// otherwise a valid `<pitch>` is required (see [`parse_pitch`]).
fn parse_note(
    note: &Node,
    divisions: f64,
    current_dynamic: Option<Dynamic>,
    current_beat: f64,
    last_note_start: f64,
    measure_number: usize,
) -> Result<ParsedNote, ScoreError> {
    let is_rest = find_descendant(note, "rest").is_some();
    let is_grace = find_descendant(note, "grace").is_some();

    let duration_divs = match find_descendant_text(note, "duration") {
        Some(s) => s.parse::<f64>().map_err(|_| {
            ScoreError::MusicXml(format!(
                "invalid <duration> '{s}' in note at measure {measure_number}"
            ))
        })?,
        None if is_grace => 0.0,
        None => {
            return Err(ScoreError::MusicXml(format!(
                "note at measure {measure_number} is missing <duration>"
            )));
        }
    };
    // `divisions` is validated > 0 before any note is parsed.
    let duration_beats = duration_divs / divisions;

    let is_chord = find_descendant(note, "chord").is_some();
    let start_beat = if is_chord {
        last_note_start
    } else {
        current_beat
    };

    let (pitch_hz, midi_number) = if is_rest {
        (0.0, 0)
    } else if let Some(pitch_node) = find_descendant(note, "pitch") {
        parse_pitch(&pitch_node, measure_number)?
    } else {
        return Err(ScoreError::MusicXml(format!(
            "note at measure {measure_number} has neither <pitch> nor <rest>"
        )));
    };

    Ok(ParsedNote {
        note: ScoreNote {
            pitch_hz,
            midi_number,
            duration_beats,
            start_beat,
            dynamic: current_dynamic,
            is_rest,
        },
        is_chord,
    })
}

/// Parse a `<pitch>` node into `(frequency_hz, midi_number)`.
///
/// A `<pitch>` must carry a valid `<step>` and `<octave>`; fabricating defaults
/// (e.g. C4) when the data is missing or malformed produces silently-wrong
/// notes that are worse than an error. `<alter>` is optional (natural = 0) but
/// an unparseable value is still an error.
fn parse_pitch(pitch_node: &Node, measure_number: usize) -> Result<(f64, u8), ScoreError> {
    let step_raw = find_descendant_text(pitch_node, "step").ok_or_else(|| {
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

    let octave_str = find_descendant_text(pitch_node, "octave").ok_or_else(|| {
        ScoreError::MusicXml(format!(
            "missing <octave> in pitch at measure {measure_number}"
        ))
    })?;
    let octave = octave_str.parse::<i8>().map_err(|_| {
        ScoreError::MusicXml(format!(
            "invalid octave '{octave_str}' at measure {measure_number}"
        ))
    })?;

    let alter = match find_descendant_text(pitch_node, "alter") {
        None => 0.0,
        Some(s) => s.parse::<f64>().map_err(|_| {
            ScoreError::MusicXml(format!("invalid alter '{s}' at measure {measure_number}"))
        })?,
    };

    let midi = pitch_to_midi(&step, octave, alter);
    Ok((midi_to_hz(midi), midi.round() as u8))
}

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
    // Scan ALL element children; don't bail on the first unknown tag.
    // MusicXML sometimes wraps dynamics in other markings (e.g. a stray
    // <other-dynamics> followed by the real <f/>), and returning early
    // on the first non-match would miss the valid one.
    for child in dynamics_node.children() {
        if !child.is_element() {
            continue;
        }
        let dyn_opt = match child.tag_name().name() {
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
        if dyn_opt.is_some() {
            return dyn_opt;
        }
    }
    None
}

/// Extract and parse the required `<duration>` child of `<forward>` /
/// `<backup>`. Both elements are defined by MusicXML to carry a positive
/// `<duration>` in divisions; missing or unparseable values indicate a
/// malformed file and must be rejected (coercing to 0 silently shifts
/// every subsequent beat cursor).
fn parse_required_duration(
    node: &Node,
    elem: &str,
    measure_number: usize,
) -> Result<f64, ScoreError> {
    let s = find_descendant_text(node, "duration").ok_or_else(|| {
        ScoreError::MusicXml(format!(
            "<{elem}> at measure {measure_number} is missing <duration>"
        ))
    })?;
    let d = s.parse::<f64>().map_err(|_| {
        ScoreError::MusicXml(format!(
            "invalid <duration> '{s}' in <{elem}> at measure {measure_number}"
        ))
    })?;
    if !(d.is_finite() && d >= 0.0) {
        return Err(ScoreError::MusicXml(format!(
            "<{elem}> at measure {measure_number} requires non-negative finite duration (got {d})"
        )));
    }
    Ok(d)
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

    #[test]
    fn chord_with_mixed_durations_shares_previous_start_beat() {
        // Regression test for the chord start_beat bug.
        //
        // Previous (buggy) logic: `start_beat = current_beat - duration_beats`.
        // For a quarter-note C (duration 4, start 0) followed by a *eighth*-note
        // <chord> E (duration 2), the buggy calc gave:
        //   current_beat = 1.0 (after quarter)
        //   duration_beats = 0.5 (eighth)
        //   start_beat = 1.0 - 0.5 = 0.5   ← WRONG, should be 0.0
        //
        // With the fix, the chord note uses last_note_start (0.0) directly.
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>4</divisions></attributes>
      <note>
        <pitch><step>C</step><octave>4</octave></pitch>
        <duration>4</duration>
      </note>
      <note>
        <chord/>
        <pitch><step>E</step><octave>4</octave></pitch>
        <duration>2</duration>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        let model = parse_musicxml_str(xml).expect("parse chord XML");
        let notes = &model.measures[0].notes;
        assert_eq!(notes.len(), 2, "Expected 2 notes in the chord measure");
        assert!(
            (notes[0].start_beat - 0.0).abs() < 1e-9,
            "Leader C should start at beat 0.0, got {}",
            notes[0].start_beat,
        );
        assert!(
            (notes[1].start_beat - 0.0).abs() < 1e-9,
            "Chord E (even with shorter duration) should share start 0.0, got {}",
            notes[1].start_beat,
        );
        // And durations must still be preserved independently.
        assert!((notes[0].duration_beats - 1.0).abs() < 1e-9);
        assert!((notes[1].duration_beats - 0.5).abs() < 1e-9);
    }

    #[test]
    fn non_chord_note_after_chord_advances_from_leader_not_chord() {
        // Harder regression: the note *after* the chord should start at
        // leader_start + leader_duration, NOT after the chord note. Because
        // chord notes don't advance `current_beat`, this should Just Work
        // provided the bookkeeping is right.
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>4</divisions></attributes>
      <note>
        <pitch><step>C</step><octave>4</octave></pitch>
        <duration>4</duration>
      </note>
      <note>
        <chord/>
        <pitch><step>E</step><octave>4</octave></pitch>
        <duration>2</duration>
      </note>
      <note>
        <pitch><step>D</step><octave>4</octave></pitch>
        <duration>4</duration>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        let model = parse_musicxml_str(xml).expect("parse XML");
        let notes = &model.measures[0].notes;
        assert_eq!(notes.len(), 3);
        // Sequence: C at 0, E (chord) at 0, D at 1 (after C's full quarter).
        assert!((notes[2].start_beat - 1.0).abs() < 1e-9);
    }

    #[test]
    fn non_positive_divisions_is_rejected() {
        // Regression: we used to silently accept <divisions>0</divisions>
        // (coercing note duration math to zero or divide-by-zero territory).
        // Reject explicitly instead.
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>0</divisions></attributes>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>4</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let err = parse_musicxml_str(xml).unwrap_err();
        match err {
            ScoreError::MusicXml(ref m) => {
                assert!(
                    m.contains("divisions"),
                    "Error should mention divisions, got: {m}"
                );
            }
            other => panic!("Expected MusicXml error, got: {other:?}"),
        }
    }

    #[test]
    fn missing_note_duration_is_rejected() {
        // Regression: missing <duration> on a regular note used to coerce
        // to 0.0 and silently produce zero-length notes.
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>4</divisions></attributes>
      <note><pitch><step>C</step><octave>4</octave></pitch></note>
    </measure>
  </part>
</score-partwise>"#;
        let err = parse_musicxml_str(xml).unwrap_err();
        match err {
            ScoreError::MusicXml(ref m) => {
                assert!(
                    m.contains("duration"),
                    "Error should mention duration, got: {m}"
                );
            }
            other => panic!("Expected MusicXml error, got: {other:?}"),
        }
    }

    #[test]
    fn parse_dynamic_scans_all_children_not_just_first() {
        // Regression: `parse_dynamic` returned on the first child, so an
        // unrecognised wrapper element would hide the real dynamic.
        // This XML puts a non-standard <other-dynamics> before the
        // legitimate <f/>; the old code missed it.
        let xml = r#"<?xml version="1.0"?>
<score-partwise>
  <part-list><score-part id="P1"><part-name>X</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>4</divisions></attributes>
      <direction><dynamics><other-dynamics>sfz</other-dynamics><f/></dynamics></direction>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>4</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let model = parse_musicxml_str(xml).expect("parse XML with mixed dynamics children");
        let note = &model.measures[0].notes[0];
        assert_eq!(
            note.dynamic,
            Some(Dynamic::F),
            "Should find the <f/> even though <other-dynamics> came first"
        );
    }
}
