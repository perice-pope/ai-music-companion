//! Exercise insights (#252 self-improvement): pure analysis over the exercise
//! log. The log records what the engine GENERATED and what came back; this
//! module answers the founder's question — "which exercises are good?" — per
//! material shape: how often it's dealt, how often it gets played to a grade,
//! whether accuracy on it is rising (it teaches) or sinking (too hard / bad).
//!
//! Read-only and deterministic; nothing here mutates the coach. Wiring the
//! verdicts back into drill selection is the follow-up issue.

use serde::{Deserialize, Serialize};

use crate::store::ExerciseLogEntry;
use variations::VariationSpec;

/// Aggregated verdict for one material shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeInsight {
    /// Canonical shape name, e.g. `"major up-down"`, `"pattern 1-2-3-5 on
    /// dorian"`, `"player cell (5 notes)"`.
    pub shape: String,
    /// Times the engine dealt this shape.
    pub generated: u32,
    /// Times it was played through to a grade — the engagement signal
    /// (dealt-but-never-graded is the "they bailed" signal).
    pub graded: u32,
    /// Mean graded accuracy, 0..1.
    pub mean_accuracy: f32,
    /// Newer-half mean minus older-half mean of the grades: positive = the
    /// shape is teaching; negative = it isn't landing. 0 below 4 grades.
    pub accuracy_trend: f32,
}

/// Canonical shape of a spec — the unit the analysis groups by.
fn shape_of(spec_json: &str) -> String {
    // Score practice logs a score reference, not a VariationSpec (#337 S4).
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(spec_json) {
        if let Some(title) = v.get("score_title").and_then(|t| t.as_str()) {
            return format!("score: {title}");
        }
    }
    let Ok(spec) = serde_json::from_str::<VariationSpec>(spec_json) else {
        return "unparseable".to_owned();
    };
    if let Some(cell) = spec.cell.as_ref().filter(|c| !c.is_empty()) {
        return format!("player cell ({} notes)", cell.len());
    }
    if let (Some(d), Some(s)) = (
        spec.degrees.as_ref().filter(|d| !d.is_empty()),
        spec.scale.as_ref(),
    ) {
        let digits: Vec<String> = d.iter().map(u8::to_string).collect();
        return format!(
            "pattern {} on {}",
            digits.join("-"),
            s.scale.label().to_lowercase()
        );
    }
    if let Some(s) = spec.scale {
        return format!("{} {}", s.scale.label().to_lowercase(), s.pattern.label());
    }
    if let Some(c) = spec.chord {
        // #349 T2b: a stacked spec deals block chords, not an arpeggio —
        // the shape must say what was actually asked (AC4).
        let motion = if c.stacked {
            "block chords"
        } else {
            "arpeggio"
        };
        return format!("{:?} {motion}", c.chord).to_lowercase();
    }
    if let Some(i) = spec.interval {
        return format!(
            "{}-semitone intervals {}",
            i.semitones,
            if i.ascending { "up" } else { "down" }
        );
    }
    "bare roots".to_owned()
}

/// Analyze the whole log (oldest → newest). Returns one insight per shape,
/// most-dealt first — stable and deterministic.
pub fn exercise_insights(log: &[ExerciseLogEntry]) -> Vec<ShapeInsight> {
    use std::collections::BTreeMap;
    let mut grades: BTreeMap<String, (u32, Vec<f32>)> = BTreeMap::new();
    for entry in log {
        let shape = shape_of(&entry.spec_json);
        let slot = grades.entry(shape).or_insert((0, Vec::new()));
        slot.0 += 1;
        if let Some(a) = entry.accuracy {
            slot.1.push(a as f32);
        }
    }
    let mut out: Vec<ShapeInsight> = grades
        .into_iter()
        .map(|(shape, (generated, accs))| {
            let graded = accs.len() as u32;
            let mean_accuracy = if accs.is_empty() {
                0.0
            } else {
                accs.iter().sum::<f32>() / accs.len() as f32
            };
            let accuracy_trend = if accs.len() >= 4 {
                let mid = accs.len() / 2;
                let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
                mean(&accs[mid..]) - mean(&accs[..mid])
            } else {
                0.0
            };
            ShapeInsight {
                shape,
                generated,
                graded,
                mean_accuracy,
                accuracy_trend,
            }
        })
        .collect();
    out.sort_by(|a, b| b.generated.cmp(&a.generated).then(a.shape.cmp(&b.shape)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(spec_json: &str, accuracy: Option<f64>) -> ExerciseLogEntry {
        ExerciseLogEntry {
            source: "lesson".to_owned(),
            label: "x".to_owned(),
            spec_json: spec_json.to_owned(),
            seed: 1,
            difficulty: 2,
            tonic: 0,
            accuracy,
        }
    }

    fn scale_spec() -> String {
        serde_json::to_string(&variations::VariationSpec {
            roots: vec![60],
            cell: None,
            degrees: None,
            progression: None,
            scale: Some(variations::ScaleModifier {
                scale: variations::ScaleType::Major,
                pattern: variations::ScalePattern::UpDown,
            }),
            chord: None,
            interval: None,
            enclosure: None,
            direction: variations::DirectionMode::Forward,
            rhythm: variations::RhythmSpec {
                notes_per_beat: 2,
                tempo_bpm: 80.0,
                rest_beats_between_roots: 1.0,
            },
            randomize_roots: false,
        })
        .unwrap()
    }

    fn cell_spec() -> String {
        let mut spec: variations::VariationSpec = serde_json::from_str(&scale_spec()).unwrap();
        spec.cell = Some(vec![0, 3, 2, 7, 5]);
        serde_json::to_string(&spec).unwrap()
    }

    /// The founder's question, answered per shape: dealt-vs-graded counts the
    /// engagement, the grade trend says whether the shape TEACHES. Fails if
    /// grouping, means, or the trend halves break.
    #[test]
    fn insights_aggregate_per_shape_with_trend() {
        let log = vec![
            entry(&scale_spec(), Some(0.4)),
            entry(&scale_spec(), Some(0.5)),
            entry(&scale_spec(), Some(0.8)),
            entry(&scale_spec(), Some(0.9)),
            entry(&scale_spec(), None), // dealt, bailed
            entry(&cell_spec(), Some(0.7)),
        ];
        let insights = exercise_insights(&log);
        assert_eq!(insights.len(), 2);
        let scale = &insights[0]; // most-dealt first
        assert_eq!(scale.shape, "major up-down");
        assert_eq!((scale.generated, scale.graded), (5, 4));
        assert!((scale.mean_accuracy - 0.65).abs() < 1e-6);
        assert!(
            (scale.accuracy_trend - 0.4).abs() < 1e-6,
            "0.85 late vs 0.45 early = the shape teaches"
        );
        let cell = &insights[1];
        assert_eq!(cell.shape, "player cell (5 notes)");
        assert_eq!(cell.accuracy_trend, 0.0, "below 4 grades = no trend claim");
    }

    /// Shape naming covers the material ladder; garbage JSON groups under
    /// "unparseable" instead of panicking.
    #[test]
    fn shapes_name_the_material_ladder() {
        let mut spec: variations::VariationSpec = serde_json::from_str(&scale_spec()).unwrap();
        spec.degrees = Some(vec![1, 2, 3, 5]);
        let pat = serde_json::to_string(&spec).unwrap();
        let log = vec![entry(&pat, None), entry("{not json", None)];
        let shapes: Vec<String> = exercise_insights(&log)
            .into_iter()
            .map(|i| i.shape)
            .collect();
        assert!(shapes.contains(&"pattern 1-2-3-5 on major".to_owned()));
        assert!(shapes.contains(&"unparseable".to_owned()));
    }

    /// #349 T2b AC4: a STACKED chord spec's shape says "block chords" — an
    /// exercise-log row must not call a block drill an arpeggio.
    #[test]
    fn stacked_specs_shape_as_block_chords() {
        let spec = variations::VariationSpec {
            roots: vec![60],
            cell: None,
            degrees: None,
            progression: None,
            scale: None,
            chord: Some(variations::ChordModifier {
                chord: variations::ChordType::Dominant7,
                pattern: variations::ArpeggioPattern::Ascending,
                inversion: 0,
                stacked: true,
            }),
            interval: None,
            enclosure: None,
            direction: variations::DirectionMode::Forward,
            rhythm: variations::RhythmSpec::default(),
            randomize_roots: false,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let log = vec![entry(&json, None)];
        let shapes: Vec<String> = exercise_insights(&log)
            .into_iter()
            .map(|i| i.shape)
            .collect();
        assert!(
            shapes.contains(&"dominant7 block chords".to_owned()),
            "got {shapes:?}"
        );
    }
}
