//! Exercise insights (#252 self-improvement): pure analysis over the exercise
//! log. The log records what the engine GENERATED and what came back; this
//! module answers the founder's question — "which exercises are good?" — per
//! material shape: how often it's dealt, how often it gets played to a grade,
//! whether accuracy on it is rising (it teaches) or sinking (too hard / bad).
//!
//! Read-only and deterministic; nothing here mutates the coach. Wiring the
//! verdicts back into drill selection is the follow-up issue.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

use crate::learner::Mastery;
use crate::store::{ExerciseLogEntry, TimedExerciseLogEntry};
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
    if let Some(p) = spec.progression.as_ref().filter(|p| !p.is_empty()) {
        // #349 T3c: the log names the shape, root-agnostic like the rest.
        let names: Vec<&str> = p.iter().map(|st| st.chord.quality().suffix()).collect();
        return format!("{}-chord progression ({})", p.len(), names.join(" "));
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

// ---------------------------------------------------------------------------
// #453 S1: the history analyzer — evidence-cited practice suggestions.
//
// Pure functions over what already accumulates locally (the timed exercise
// log + key_mastery EWMAs). Every suggestion embeds its numbers in its text
// and carries a compact citation; below any evidence bar a rule returns
// NOTHING — silence > lies, the #445-6b thin-recap discipline verbatim.
// ---------------------------------------------------------------------------

/// What flavour of history a suggestion is grounded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionKind {
    /// A key you keep playing and it isn't landing.
    Trend,
    /// A whole tonic side gone dark while you practiced elsewhere.
    Neglect,
    /// A cell of yours that jumped — push it.
    Momentum,
}

/// One history-grounded suggestion. `text` is the human sentence and embeds
/// its own numbers (counts, days, percentages); `evidence` is the compact
/// citation (keys/rows, raw values, dates) for anything downstream that wants
/// to show its work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PracticeSuggestion {
    pub kind: SuggestionKind,
    pub text: String,
    pub evidence: String,
}

/// Trend bar: below 5 graded drills the EWMA (alpha 0.3) is mostly priming
/// noise — and it keeps the trend bar stricter than the wheel's own
/// `OWNED_MIN_ATTEMPTS` (3), so we never claim a struggle on thinner
/// evidence than "owned" needs.
pub const TREND_MIN_ATTEMPTS: u32 = 5;
/// Trend bar: the founder's number ("below 60%"). Far under
/// `OWNED_ACCURACY_THRESHOLD` (0.85), leaving a quiet middle where neither
/// praise nor concern fires.
pub const TREND_ACCURACY_BAR: f32 = 0.6;
/// Trend bar: the key must have been attempted within this many days — a
/// weak key you stopped playing is history, not a live trend.
pub const TREND_RECENT_DAYS: i64 = 14;

/// Neglect bar: the founder's "two weeks".
pub const NEGLECT_MIN_DAYS: i64 = 14;
/// Neglect bar: while the group sat idle, at least this many rows must have
/// landed in OTHER pitch classes inside the window — without the contrast
/// the whole log is idle and silence wins (an absent player owes no keys).
pub const NEGLECT_CONTRAST_MIN_ROWS: usize = 8;
/// The tonic groups neglect watches. C (0) and F#/Gb (6) sit on the seam of
/// both signature sides and are claimed by neither — either claim about
/// them would be contestable.
const NEGLECT_GROUPS: [(&str, [u8; 5]); 2] = [
    ("the flat keys", [1, 3, 5, 8, 10]),
    ("the sharp keys", [2, 4, 7, 9, 11]),
];

/// Momentum bar (K): newer-half vs older-half means need ≥3 rows each so
/// neither half rests on one lucky take (the same halving discipline as
/// [`ShapeInsight::accuracy_trend`], which refuses below 4).
pub const MOMENTUM_MIN_GRADED: usize = 6;
/// Momentum window: the delta is computed over the cell's most recent ≤12
/// graded rows — the claim is about now, not the cell's lifetime.
pub const MOMENTUM_WINDOW_ROWS: usize = 12;
/// Momentum bar (X): 15 points clears normal grade jitter (the issue's
/// 55%→80% example clears it three times over).
pub const MOMENTUM_MIN_DELTA: f32 = 0.15;
/// Momentum bar: the newest graded row must be within this many days — no
/// celebrating ancient history.
pub const MOMENTUM_RECENT_DAYS: i64 = 14;

/// `logged_at` is stored TEXT — a copied/edited/corrupt database may hold
/// anything, so parse defensively; a row whose stamp fails feeds no rule.
fn parse_stamp(logged_at: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(logged_at).ok()
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len() as f32
}

fn pct(v: f32) -> i32 {
    (v * 100.0).round() as i32
}

fn days_phrase(days: i64) -> String {
    match days {
        0 => "today".to_owned(),
        1 => "1 day ago".to_owned(),
        n => format!("{n} days ago"),
    }
}

/// `"3:major"` → `"Eb major"`, through the same signature-driven spelling
/// voice everything else uses (#335). Unparseable keys return `None` — a
/// key we can't name honestly is a key we don't make claims about.
fn mastery_key_display(key: &str) -> Option<String> {
    let (tonic_str, mode) = key.split_once(':')?;
    let tonic: u8 = tonic_str.parse().ok()?;
    if tonic >= 12 || mode.is_empty() {
        return None;
    }
    let fifths = crate::coach::key_signature_for(tonic, mode).fifths;
    Some(format!(
        "{} {mode}",
        crate::coach::tonic_display_name(tonic, fifths)
    ))
}

fn trend_suggestions(
    key_mastery: &BTreeMap<String, Mastery>,
    now: DateTime<Utc>,
) -> Vec<PracticeSuggestion> {
    let mut out = Vec::new();
    // BTreeMap iteration = sorted by mastery key = deterministic order.
    for (key, m) in key_mastery {
        if !m.accuracy_ewma.is_finite()
            || m.attempts < TREND_MIN_ATTEMPTS
            || m.accuracy_ewma >= TREND_ACCURACY_BAR
        {
            continue;
        }
        let age_secs = now.timestamp() - m.last_epoch_secs;
        if age_secs > TREND_RECENT_DAYS * 86_400 {
            continue; // not a live trend
        }
        let Some(name) = mastery_key_display(key) else {
            continue;
        };
        let days = (age_secs.max(0)) / 86_400;
        out.push(PracticeSuggestion {
            kind: SuggestionKind::Trend,
            text: format!(
                "Your {name} rows are sitting at {}% over {} attempts (last one {}) — worth a slow pass.",
                pct(m.accuracy_ewma),
                m.attempts,
                days_phrase(days),
            ),
            evidence: format!(
                "key_mastery {key}: {} attempts, accuracy EWMA {:.2}, last attempt {}d ago",
                m.attempts, m.accuracy_ewma, days,
            ),
        });
    }
    out
}

fn neglect_suggestions(
    rows: &[(DateTime<FixedOffset>, &ExerciseLogEntry)],
    now: DateTime<Utc>,
) -> Vec<PracticeSuggestion> {
    let mut out = Vec::new();
    // min() by stamp, not first-by-id: a clock rollback could disorder
    // stamps against insertion order and the span claim must stay honest.
    let Some(oldest) = rows.iter().map(|(t, _)| *t).min() else {
        return out; // empty log: nothing was neglected, nothing to say
    };
    let log_span_days = now.signed_duration_since(oldest).num_days();
    for (group_name, tonics) in NEGLECT_GROUPS {
        let in_group = |e: &ExerciseLogEntry| tonics.contains(&(e.tonic % 12));
        let last_in_group = rows
            .iter()
            .filter(|(_, e)| in_group(e))
            .map(|(t, _)| *t)
            .max();
        // The contrast: rows in OTHER pitch classes inside the idle window.
        let contrast = rows
            .iter()
            .filter(|(t, e)| {
                !in_group(e) && now.signed_duration_since(*t).num_days() < NEGLECT_MIN_DAYS
            })
            .count();
        if contrast < NEGLECT_CONTRAST_MIN_ROWS {
            continue;
        }
        match last_in_group {
            Some(last) => {
                let days = now.signed_duration_since(last).num_days();
                if days < NEGLECT_MIN_DAYS {
                    continue;
                }
                out.push(PracticeSuggestion {
                    kind: SuggestionKind::Neglect,
                    text: format!(
                        "You haven't rowed anything in {group_name} for {days} days, while {contrast} rows landed elsewhere — worth a pass through them.",
                    ),
                    evidence: format!(
                        "last {group_name} row {} ({days}d ago); {contrast} rows in other keys in the last {NEGLECT_MIN_DAYS}d",
                        last.format("%Y-%m-%d"),
                    ),
                });
            }
            None => {
                // Never practiced: only citable once the log itself spans the
                // window — a week-old log can't owe anyone two weeks.
                if log_span_days < NEGLECT_MIN_DAYS {
                    continue;
                }
                out.push(PracticeSuggestion {
                    kind: SuggestionKind::Neglect,
                    text: format!(
                        "You haven't rowed anything in {group_name} yet — not once in your {log_span_days}-day log, while {contrast} recent rows landed elsewhere.",
                    ),
                    evidence: format!(
                        "no {group_name} rows in {} logged rows since {} ({log_span_days}d); {contrast} rows in other keys in the last {NEGLECT_MIN_DAYS}d",
                        rows.len(),
                        oldest.format("%Y-%m-%d"),
                    ),
                });
            }
        }
    }
    out
}

/// One graded row of a cell: (write stamp, accuracy).
type CellGrade = (DateTime<FixedOffset>, f32);

fn momentum_suggestions(
    rows: &[(DateTime<FixedOffset>, &ExerciseLogEntry)],
    now: DateTime<Utc>,
) -> Vec<PracticeSuggestion> {
    // Graded rows per cell, in log order. First-appearance order of the
    // cells is itself deterministic given the same log.
    let mut cells: Vec<(Vec<i8>, Vec<CellGrade>)> = Vec::new();
    for (t, e) in rows {
        let Some(a) = e.accuracy else { continue };
        let Ok(spec) = serde_json::from_str::<VariationSpec>(&e.spec_json) else {
            continue;
        };
        // The same line My Patterns draws: catalog drills aren't "your"
        // cells, and a single note isn't a pattern.
        let Some(cell) = spec.cell.filter(|c| c.len() >= 2) else {
            continue;
        };
        match cells.iter_mut().find(|(c, _)| *c == cell) {
            Some((_, grades)) => grades.push((*t, a as f32)),
            None => cells.push((cell, vec![(*t, a as f32)])),
        }
    }
    let mut out = Vec::new();
    for (cell, grades) in &cells {
        let window = &grades[grades.len().saturating_sub(MOMENTUM_WINDOW_ROWS)..];
        if window.len() < MOMENTUM_MIN_GRADED {
            continue;
        }
        let (newest, _) = window[window.len() - 1];
        if now.signed_duration_since(newest).num_days() > MOMENTUM_RECENT_DAYS {
            continue;
        }
        let accs: Vec<f32> = window.iter().map(|(_, a)| *a).collect();
        let mid = accs.len() / 2;
        let (early, late) = (mean(&accs[..mid]), mean(&accs[mid..]));
        if late - early < MOMENTUM_MIN_DELTA {
            continue;
        }
        let offsets = cell.iter().map(i8::to_string).collect::<Vec<_>>().join(" ");
        out.push(PracticeSuggestion {
            kind: SuggestionKind::Momentum,
            text: format!(
                "Your {}-note cell ({offsets}) climbed from {}% to {}% across its last {} graded rows — push the tempo.",
                cell.len(),
                pct(early),
                pct(late),
                window.len(),
            ),
            evidence: format!(
                "cell [{offsets}]: older-half mean {early:.2} → newer-half mean {late:.2} over {} graded rows, newest {}",
                window.len(),
                newest.format("%Y-%m-%d"),
            ),
        });
    }
    out
}

/// The #453 S1 analyzer: evidence-cited suggestions from the timed exercise
/// log + `key_mastery`. Pure and deterministic — `now` is injected. Rows
/// whose `logged_at` doesn't parse as RFC3339 feed no rule (a corrupt stamp
/// must never invent a claim). Order is pinned: Trend (by mastery key),
/// Neglect (fixed group order), Momentum (cell first-appearance).
pub fn practice_suggestions(
    log: &[TimedExerciseLogEntry],
    key_mastery: &BTreeMap<String, Mastery>,
    now: DateTime<Utc>,
) -> Vec<PracticeSuggestion> {
    let rows: Vec<(DateTime<FixedOffset>, &ExerciseLogEntry)> = log
        .iter()
        .filter_map(|r| parse_stamp(&r.logged_at).map(|t| (t, &r.entry)))
        .collect();
    let mut out = trend_suggestions(key_mastery, now);
    out.extend(neglect_suggestions(&rows, now));
    out.extend(momentum_suggestions(&rows, now));
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

    /// #349 T3c: a progression spec's shape names the shape — deleting the
    /// arm would regress the exercise log to a scale/roots misreport with
    /// green CI (review N5).
    #[test]
    fn progression_specs_shape_as_progressions() {
        let spec = variations::VariationSpec {
            roots: vec![62],
            cell: None,
            degrees: None,
            progression: Some(vec![
                variations::ProgressionStep {
                    offset: 0,
                    chord: variations::ChordType::Minor7,
                },
                variations::ProgressionStep {
                    offset: 5,
                    chord: variations::ChordType::Dominant7,
                },
                variations::ProgressionStep {
                    offset: 10,
                    chord: variations::ChordType::Major7,
                },
            ]),
            scale: None,
            chord: None,
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
            shapes.contains(&"3-chord progression (m7 7 maj7)".to_owned()),
            "got {shapes:?}"
        );
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

    // -----------------------------------------------------------------
    // #453 S1: the history analyzer.
    // -----------------------------------------------------------------

    use chrono::TimeZone;

    /// A fixed "now" so every claim about days is exact.
    fn test_now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap()
    }

    fn timed(
        spec_json: &str,
        tonic: u8,
        accuracy: Option<f64>,
        days_ago: i64,
    ) -> TimedExerciseLogEntry {
        TimedExerciseLogEntry {
            logged_at: (test_now() - chrono::Duration::days(days_ago)).to_rfc3339(),
            entry: ExerciseLogEntry {
                source: "explore".to_owned(),
                label: "x".to_owned(),
                spec_json: spec_json.to_owned(),
                seed: 1,
                difficulty: 2,
                tonic,
                accuracy,
            },
        }
    }

    fn mastery_row(attempts: u32, ewma: f32, days_ago: i64) -> Mastery {
        Mastery {
            attempts,
            accuracy_ewma: ewma,
            owned: false,
            last_epoch_secs: test_now().timestamp() - days_ago * 86_400,
            extra: Default::default(),
        }
    }

    fn km(entries: &[(&str, Mastery)]) -> BTreeMap<String, Mastery> {
        entries
            .iter()
            .map(|(k, m)| ((*k).to_owned(), m.clone()))
            .collect()
    }

    /// AC1: Trend fires only above EVERY bar (≥5 attempts, EWMA <0.60,
    /// last attempt ≤14 days) — dropping any single bar silences it, the
    /// boundaries are pinned, and the text carries the numbers. Fails if a
    /// bar loosens or the suggestion stops citing its evidence.
    #[test]
    fn trend_fires_only_above_every_bar() {
        let now = test_now();
        let fires = practice_suggestions(&[], &km(&[("3:major", mastery_row(6, 0.54, 2))]), now);
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].kind, SuggestionKind::Trend);
        assert!(
            fires[0].text.contains("Eb major")
                && fires[0].text.contains("54%")
                && fires[0].text.contains("6 attempts")
                && fires[0].text.contains("2 days ago"),
            "text embeds key, percentage, count, days: {}",
            fires[0].text
        );
        assert!(
            fires[0].evidence.contains("3:major"),
            "evidence cites the mastery key: {}",
            fires[0].evidence
        );

        // Boundaries: ≥5 attempts and ≤14 days are inclusive.
        let boundary =
            practice_suggestions(&[], &km(&[("3:major", mastery_row(5, 0.54, 14))]), now);
        assert_eq!(boundary.len(), 1, "5 attempts / 14 days is still evidence");

        // Each bar individually silences.
        for (name, m) in [
            ("too few attempts", mastery_row(4, 0.54, 2)),
            (
                "ewma at the bar (0.60 is not below it)",
                mastery_row(6, 0.60, 2),
            ),
            ("stale (15 days)", mastery_row(6, 0.54, 15)),
            ("non-finite ewma", mastery_row(6, f32::NAN, 2)),
        ] {
            assert!(
                practice_suggestions(&[], &km(&[("3:major", m)]), now).is_empty(),
                "{name} must stay silent"
            );
        }
        // A key that can't be named honestly makes no claim.
        assert!(
            practice_suggestions(&[], &km(&[("garbage", mastery_row(6, 0.5, 2))]), now).is_empty()
        );
    }

    /// AC2: Neglect needs BOTH the ≥14-day absence and the ≥8-row contrast
    /// elsewhere inside the window; a recently-practiced group stays quiet;
    /// the never-practiced form needs the log itself to span the window.
    /// Fails if either bar drops or old rows start counting as contrast.
    #[test]
    fn neglect_needs_absence_and_the_contrast() {
        let now = test_now();
        let no_mastery = BTreeMap::new();
        let sharp_rows = |n: usize, days_ago: i64| -> Vec<TimedExerciseLogEntry> {
            (0..n)
                .map(|_| timed(&scale_spec(), 7, None, days_ago))
                .collect()
        };

        // Flat row 20 days ago + 8 recent sharp rows → flat neglect fires,
        // sharp keys (practiced 3 days ago) stay quiet.
        let mut log = vec![timed(&scale_spec(), 10, None, 20)];
        log.extend(sharp_rows(8, 3));
        let out = practice_suggestions(&log, &no_mastery, now);
        assert_eq!(out.len(), 1, "exactly the flat-key neglect: {out:?}");
        assert_eq!(out[0].kind, SuggestionKind::Neglect);
        assert!(
            out[0].text.contains("the flat keys")
                && out[0].text.contains("20 days")
                && out[0].text.contains("8 rows"),
            "text embeds group, days, contrast: {}",
            out[0].text
        );

        // Only 7 contrast rows → silence.
        let mut thin = vec![timed(&scale_spec(), 10, None, 20)];
        thin.extend(sharp_rows(7, 3));
        assert!(practice_suggestions(&thin, &no_mastery, now).is_empty());

        // 8 rows elsewhere but OUTSIDE the window → no contrast → silence.
        let mut stale = vec![timed(&scale_spec(), 10, None, 20)];
        stale.extend(sharp_rows(8, 15));
        assert!(practice_suggestions(&stale, &no_mastery, now).is_empty());

        // Never-practiced form: no flat row ever, log spans 20 days →
        // the "not once" claim, still citing the contrast.
        let mut never = sharp_rows(1, 20);
        never.extend(sharp_rows(8, 3));
        let out = practice_suggestions(&never, &no_mastery, now);
        assert_eq!(out.len(), 1);
        assert!(
            out[0].text.contains("the flat keys") && out[0].text.contains("not once"),
            "the never form: {}",
            out[0].text
        );

        // A log younger than the window can never owe two weeks.
        let young = sharp_rows(9, 3);
        assert!(practice_suggestions(&young, &no_mastery, now).is_empty());
    }

    /// AC3: Momentum needs ≥6 graded rows for ONE cell, a ≥+0.15 half-delta
    /// inside the 12-row window, and a newest row ≤14 days old; catalog
    /// drills never earn it. Fails if K drops, the delta bar loosens, the
    /// window leaks lifetime rows, or ungraded rows start counting.
    #[test]
    fn momentum_needs_k_rows_delta_and_recency() {
        let now = test_now();
        let no_mastery = BTreeMap::new();
        let rising: Vec<f64> = vec![0.5, 0.5, 0.5, 0.55, 0.75, 0.8, 0.8, 0.85];
        let cell_log = |accs: &[f64], newest_days_ago: i64| -> Vec<TimedExerciseLogEntry> {
            accs.iter()
                .enumerate()
                .map(|(i, &a)| {
                    let days = newest_days_ago + (accs.len() - 1 - i) as i64;
                    timed(&cell_spec(), 0, Some(a), days)
                })
                .collect()
        };

        let out = practice_suggestions(&cell_log(&rising, 1), &no_mastery, now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, SuggestionKind::Momentum);
        assert!(
            out[0].text.contains("51%")
                && out[0].text.contains("80%")
                && out[0].text.contains("8 graded rows"),
            "text embeds both halves and the count: {}",
            out[0].text
        );
        assert!(
            out[0].evidence.contains("0 3 2 7 5"),
            "evidence names the cell: {}",
            out[0].evidence
        );

        // Below K (5 graded rows) → silence, even with a huge jump.
        let five = practice_suggestions(&cell_log(&rising[3..], 1), &no_mastery, now);
        assert!(five.is_empty(), "5 graded rows is below K: {five:?}");

        // Flat accuracy → silence.
        assert!(
            practice_suggestions(&cell_log(&[0.6; 8], 1), &no_mastery, now).is_empty(),
            "no delta, no claim"
        );

        // Newest row 20 days old → ancient history, silence.
        assert!(practice_suggestions(&cell_log(&rising, 20), &no_mastery, now).is_empty());

        // The 12-row window: 8 ancient 0.9s before 12 recent flat 0.5s —
        // lifetime rows must not leak a fake (negative or positive) delta.
        // (This 20-day log legitimately earns NEGLECT for both key groups —
        // every row is tonic 0 — so pin the absence of MOMENTUM only.)
        let mut lifetime = vec![0.9; 8];
        lifetime.extend(vec![0.5; 12]);
        assert!(
            practice_suggestions(&cell_log(&lifetime, 1), &no_mastery, now)
                .iter()
                .all(|s| s.kind != SuggestionKind::Momentum)
        );

        // Ungraded rows never count toward K.
        let ungraded: Vec<TimedExerciseLogEntry> =
            (0..8).map(|i| timed(&cell_spec(), 0, None, i)).collect();
        assert!(practice_suggestions(&ungraded, &no_mastery, now).is_empty());

        // Catalog drills (no cell) with the same rise → not "your" cell.
        let catalog: Vec<TimedExerciseLogEntry> = rising
            .iter()
            .enumerate()
            .map(|(i, &a)| timed(&scale_spec(), 0, Some(a), (rising.len() - 1 - i) as i64))
            .collect();
        assert!(practice_suggestions(&catalog, &no_mastery, now).is_empty());
    }

    /// AC4: empty history → empty vec; garbage `logged_at` stamps and junk
    /// spec_json feed NO rule — a neglect/momentum that would fire off
    /// well-formed stamps stays silent when the stamps are corrupt.
    #[test]
    fn empty_and_garbage_history_stay_silent() {
        let now = test_now();
        assert!(practice_suggestions(&[], &BTreeMap::new(), now).is_empty());

        // The AC2 firing shape, but every stamp is garbage → silence.
        let mut garbage: Vec<TimedExerciseLogEntry> = vec![timed(&scale_spec(), 10, None, 20)];
        garbage.extend((0..8).map(|_| timed(&scale_spec(), 7, None, 3)));
        for row in &mut garbage {
            row.logged_at = "not-a-timestamp".to_owned();
        }
        assert!(practice_suggestions(&garbage, &BTreeMap::new(), now).is_empty());

        // Junk spec_json rows with grades never become momentum.
        let junk: Vec<TimedExerciseLogEntry> = (0..8)
            .map(|i| timed("{not json", 0, Some(0.4 + 0.05 * i as f64), 7 - i))
            .collect();
        assert!(practice_suggestions(&junk, &BTreeMap::new(), now).is_empty());
    }

    /// AC5: identical inputs → identical output, order pinned Trend →
    /// Neglect → Momentum, and every suggestion carries digit-bearing text
    /// AND evidence. Fails if iteration order becomes nondeterministic or a
    /// rule stops citing.
    #[test]
    fn suggestions_are_deterministic_and_ordered() {
        let now = test_now();
        let mastery = km(&[("3:major", mastery_row(6, 0.5, 2))]);
        // Momentum cell + flat-key neglect (contrast = the cell rows, tonic 7).
        let mut log: Vec<TimedExerciseLogEntry> = vec![timed(&scale_spec(), 10, None, 20)];
        for (i, a) in [0.5, 0.5, 0.5, 0.5, 0.8, 0.8, 0.8, 0.8].iter().enumerate() {
            log.push(timed(&cell_spec(), 7, Some(*a), 8 - i as i64));
        }
        let a = practice_suggestions(&log, &mastery, now);
        let b = practice_suggestions(&log, &mastery, now);
        assert_eq!(a, b, "same inputs, same output");
        let kinds: Vec<SuggestionKind> = a.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SuggestionKind::Trend,
                SuggestionKind::Neglect,
                SuggestionKind::Momentum
            ]
        );
        for s in &a {
            assert!(
                s.text.chars().any(|c| c.is_ascii_digit())
                    && s.evidence.chars().any(|c| c.is_ascii_digit()),
                "every suggestion embeds its numbers: {s:?}"
            );
        }
    }
}
