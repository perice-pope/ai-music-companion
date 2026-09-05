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
    /// A catalog shape whose grades are climbing — deal it again (#303).
    Working,
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

// ---------------------------------------------------------------------------
// #303 S1: shape verdicts — the coach-selection contract. Per CATALOG shape
// (player cells are the Momentum rule's beat; score-reference and unparseable
// rows earn no call), the log's recent window answers: is this material
// WORKING (grades climbing — deal it again), or does it keep getting dealt
// and BAILED on (selection should rest it)? Selection wiring (S2/S3)
// consumes these; the Working call also speaks one recap line below.
// ---------------------------------------------------------------------------

/// A verdict is about the shape's recent life, not its lifetime — the same
/// 12-row discipline as [`MOMENTUM_WINDOW_ROWS`].
pub const SHAPE_WINDOW_ROWS: usize = 12;
/// Working bar (K): ≥6 graded rows in the window so each half rests on ≥3 —
/// the halving discipline of [`MOMENTUM_MIN_GRADED`].
pub const WORKING_MIN_GRADED: usize = 6;
/// Working bar (delta): 15 points clears normal grade jitter, the same bar
/// as [`MOMENTUM_MIN_DELTA`] — one climb standard, two materials.
pub const WORKING_MIN_DELTA: f32 = 0.15;
/// Both calls demand a row this recent — a verdict on abandoned material is
/// history, not guidance.
pub const SHAPE_RECENT_DAYS: i64 = 14;
/// Bailing bar: below this many deals in the window, "repeatedly dealt" is
/// an overclaim.
pub const BAILING_MIN_DEALT: usize = 6;

/// The call on one shape. All counts are WINDOW counts (the shape's most
/// recent ≤ [`SHAPE_WINDOW_ROWS`] rows), never lifetime totals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeCall {
    /// Grades rising inside the window — deal it again.
    Working { early: f32, late: f32, graded: u32 },
    /// Dealt repeatedly, rarely played to a grade — rest it.
    Bailing { dealt: u32, graded: u32 },
}

/// One shape's verdict. `newest` is the stamp the recency bar accepted
/// (newest graded row for Working, newest row for Bailing).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeVerdict {
    pub shape: String,
    pub call: ShapeCall,
    pub newest: DateTime<FixedOffset>,
}

/// The catalog gate: `Some(shape)` only for material the coach itself deals.
/// Player material — a lifted cell (Momentum's claim) or a lifted
/// progression — earns no shape verdict (one material, one voice; and S2's
/// seeded catalog dealer couldn't re-deal it anyway), a score reference is a
/// piece rather than a shape, and junk earns no call.
fn catalog_shape(spec_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(spec_json).ok()?;
    if v.get("score_title").and_then(|t| t.as_str()).is_some() {
        return None;
    }
    let spec: VariationSpec = serde_json::from_str(spec_json).ok()?;
    if spec.cell.as_ref().is_some_and(|c| !c.is_empty())
        || spec.progression.as_ref().is_some_and(|p| !p.is_empty())
    {
        return None;
    }
    Some(shape_of(spec_json))
}

/// Judge every catalog shape in the log. Pure and deterministic — shapes in
/// first-appearance order, `now` injected; corrupt stamps feed no verdict.
/// The two calls are mutually exclusive by arithmetic: Working needs ≥6
/// graded rows, Bailing allows at most window/3 (12/3 = 4).
pub fn shape_verdicts(log: &[TimedExerciseLogEntry], now: DateTime<Utc>) -> Vec<ShapeVerdict> {
    // One log row of a shape: (write stamp, grade if played through).
    type ShapeRow = (DateTime<FixedOffset>, Option<f32>);
    let mut shapes: Vec<(String, Vec<ShapeRow>)> = Vec::new();
    for r in log {
        let Some(t) = parse_stamp(&r.logged_at) else {
            continue;
        };
        let Some(shape) = catalog_shape(&r.entry.spec_json) else {
            continue;
        };
        let acc = r.entry.accuracy.map(|a| a as f32);
        match shapes.iter_mut().find(|(s, _)| *s == shape) {
            Some((_, rows)) => rows.push((t, acc)),
            None => shapes.push((shape, vec![(t, acc)])),
        }
    }
    let mut out = Vec::new();
    for (shape, rows) in shapes {
        let window = &rows[rows.len().saturating_sub(SHAPE_WINDOW_ROWS)..];
        let graded: Vec<f32> = window.iter().filter_map(|(_, a)| *a).collect();
        if graded.len() >= WORKING_MIN_GRADED {
            let newest_graded = window
                .iter()
                .rev()
                .find_map(|(t, a)| a.map(|_| *t))
                .expect("graded is non-empty");
            if now.signed_duration_since(newest_graded).num_days() > SHAPE_RECENT_DAYS {
                continue;
            }
            let mid = graded.len() / 2;
            let (early, late) = (mean(&graded[..mid]), mean(&graded[mid..]));
            if late - early >= WORKING_MIN_DELTA {
                out.push(ShapeVerdict {
                    shape,
                    call: ShapeCall::Working {
                        early,
                        late,
                        graded: graded.len() as u32,
                    },
                    newest: newest_graded,
                });
            }
            // Enough grades but no climb: neither call — grading a lot
            // isn't bailing, and a plateau isn't progress.
        } else if window.len() >= BAILING_MIN_DEALT && graded.len() * 3 <= window.len() {
            let newest = window.last().map(|(t, _)| *t).expect("window non-empty");
            if now.signed_duration_since(newest).num_days() <= SHAPE_RECENT_DAYS {
                out.push(ShapeVerdict {
                    shape,
                    call: ShapeCall::Bailing {
                        dealt: window.len() as u32,
                        graded: graded.len() as u32,
                    },
                    newest,
                });
            }
        }
    }
    out
}

/// The founder's recap line (#303): each Working verdict, spoken. Bailing
/// stays out of the recap — retirement is selection's business (S2), not a
/// nag.
fn working_suggestions(
    log: &[TimedExerciseLogEntry],
    now: DateTime<Utc>,
) -> Vec<PracticeSuggestion> {
    shape_verdicts(log, now)
        .into_iter()
        .filter_map(|v| match v.call {
            ShapeCall::Working {
                early,
                late,
                graded,
            } => Some(PracticeSuggestion {
                kind: SuggestionKind::Working,
                text: format!(
                    "Your {} rows climbed from {}% to {}% across the last {graded} graded — that material is working for you, deal it again.",
                    v.shape,
                    pct(early),
                    pct(late),
                ),
                evidence: format!(
                    "shape '{}': older-half mean {early:.2} → newer-half mean {late:.2} over {graded} graded rows, newest {}",
                    v.shape,
                    v.newest.format("%Y-%m-%d"),
                ),
            }),
            ShapeCall::Bailing { .. } => None,
        })
        .collect()
}

/// The #453 S1 analyzer: evidence-cited suggestions from the timed exercise
/// log + `key_mastery`. Pure and deterministic — `now` is injected. Rows
/// whose `logged_at` doesn't parse as RFC3339 feed no rule (a corrupt stamp
/// must never invent a claim). Order is pinned: Trend (by mastery key),
/// Neglect (fixed group order), Momentum (cell first-appearance), Working
/// (shape first-appearance, #303).
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
    out.extend(working_suggestions(log, now));
    out
}

/// #453 S2: render suggestions as a GROUNDED INPUT block for the LLM recap
/// prompt — the same posture as [`crate::idiom_recap::idiom_prompt_block`]:
/// locally computed facts the model may narrate, hedged, but never assert
/// beyond or extend. Empty string when there are no suggestions (no block,
/// no claim — the prompt stays byte-identical to today's).
pub fn history_prompt_block(suggestions: &[PracticeSuggestion]) -> String {
    if suggestions.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = suggestions
        .iter()
        .map(|s| format!("  - {} (evidence: {})", s.text, s.evidence))
        .collect();
    format!(
        "\n\nPractice-history suggestions (GROUNDED INPUT — computed locally from \
         this student's own exercise log and key mastery, each citing its \
         evidence; you MAY weave them into the next-session suggestions in your \
         own warm voice, but keep every number exactly as given, NEVER invent \
         further history claims, and NEVER present these as something you heard \
         in today's playing):\n{}",
        lines.join("\n")
    )
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

        // Review MF1 (round 1): the same window pin with the polarity that
        // actually kills the lifetime mutant — 8 ancient 0.2s then 12
        // recent FLAT 0.7s. Over the window (all 0.7) the delta is 0 →
        // silent; over the LIFETIME rows (`&grades[..]`) the halves read
        // 0.30 → 0.70 and a false "climbed from 30% to 70%" fires. (The
        // 20-day tonic-0 log again earns honest neglect — pin the absence
        // of MOMENTUM.)
        let mut plateau = vec![0.2; 8];
        plateau.extend(vec![0.7; 12]);
        assert!(
            practice_suggestions(&cell_log(&plateau, 1), &no_mastery, now)
                .iter()
                .all(|s| s.kind != SuggestionKind::Momentum),
            "ancient lows before a recent plateau must not fake a climb"
        );

        // Ungraded rows never count toward K.
        let ungraded: Vec<TimedExerciseLogEntry> =
            (0..8).map(|i| timed(&cell_spec(), 0, None, i)).collect();
        assert!(practice_suggestions(&ungraded, &no_mastery, now).is_empty());

        // Catalog drills (no cell) with the same rise → not "your" cell:
        // never Momentum. (The rise IS claimed — by #303's Working rule,
        // whose own tests pin it.)
        let catalog: Vec<TimedExerciseLogEntry> = rising
            .iter()
            .enumerate()
            .map(|(i, &a)| timed(&scale_spec(), 0, Some(a), (rising.len() - 1 - i) as i64))
            .collect();
        assert!(practice_suggestions(&catalog, &no_mastery, now)
            .iter()
            .all(|s| s.kind != SuggestionKind::Momentum));
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

        // Review MF2 (round 1): MIXED garbage — corrupt stamps among
        // well-formed rows must neither fabricate a claim nor suppress one.
        //
        // (a) ONE garbage-stamped flat row + 8 well-formed sharp rows 3
        // days ago → EMPTY. A parse fallback to some ancient default (the
        // epoch) would turn the corrupt row into a flat row "played long
        // ago" and fabricate a 14-day absence the 3-day log never earned.
        let mut one_bad_flat: Vec<TimedExerciseLogEntry> = vec![timed(&scale_spec(), 10, None, 20)];
        one_bad_flat[0].logged_at = "not-a-timestamp".to_owned();
        one_bad_flat.extend((0..8).map(|_| timed(&scale_spec(), 7, None, 3)));
        assert!(
            practice_suggestions(&one_bad_flat, &BTreeMap::new(), now).is_empty(),
            "a corrupt stamp must not fabricate ancient absence"
        );

        // (b) The EARNED neglect (flat row 20 days ago + 8 recent sharp
        // rows) with a garbage-stamped flat row mixed in → exactly the one
        // suggestion. A parse fallback to `now` would read the corrupt row
        // as "flat keys played today" and silence the honest claim.
        let mut mixed: Vec<TimedExerciseLogEntry> = vec![timed(&scale_spec(), 10, None, 20)];
        let mut bad_flat = timed(&scale_spec(), 10, None, 0);
        bad_flat.logged_at = "not-a-timestamp".to_owned();
        mixed.push(bad_flat);
        mixed.extend((0..8).map(|_| timed(&scale_spec(), 7, None, 3)));
        let out = practice_suggestions(&mixed, &BTreeMap::new(), now);
        assert_eq!(
            out.len(),
            1,
            "the earned neglect survives the corrupt row: {out:?}"
        );
        assert_eq!(out[0].kind, SuggestionKind::Neglect);
        assert!(
            out[0].text.contains("the flat keys") && out[0].text.contains("20 days"),
            "still the same cited claim: {}",
            out[0].text
        );
    }

    /// AC5 (#453) + AC6 (#303): identical inputs → identical output, order
    /// pinned Trend → Neglect → Momentum → Working, and every suggestion
    /// carries digit-bearing text AND evidence. Fails if iteration order
    /// becomes nondeterministic or a rule stops citing.
    #[test]
    fn suggestions_are_deterministic_and_ordered() {
        let now = test_now();
        let mastery = km(&[("3:major", mastery_row(6, 0.5, 2))]);
        // Momentum cell + flat-key neglect (contrast = the recent rows,
        // tonic 7) + a rising catalog scale for Working.
        let mut log: Vec<TimedExerciseLogEntry> = vec![timed(&scale_spec(), 10, None, 20)];
        for (i, a) in [0.5, 0.5, 0.5, 0.5, 0.8, 0.8, 0.8, 0.8].iter().enumerate() {
            log.push(timed(&cell_spec(), 7, Some(*a), 8 - i as i64));
            log.push(timed(&scale_spec(), 7, Some(*a), 8 - i as i64));
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
                SuggestionKind::Momentum,
                SuggestionKind::Working
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

    // -----------------------------------------------------------------
    // #303 S1: shape verdicts + the Working recap line.
    // -----------------------------------------------------------------

    /// One graded catalog row per accuracy, newest last, `newest_days_ago`
    /// for the final row — the Working twin of momentum's `cell_log`.
    fn catalog_log(accs: &[f64], newest_days_ago: i64) -> Vec<TimedExerciseLogEntry> {
        accs.iter()
            .enumerate()
            .map(|(i, &a)| {
                let days = newest_days_ago + (accs.len() - 1 - i) as i64;
                timed(&scale_spec(), 0, Some(a), days)
            })
            .collect()
    }

    /// AC1: a rising catalog shape earns exactly one Working line whose text
    /// embeds the shape, both halves, and the graded count, and whose
    /// evidence cites the halves and newest date. Fails if the bars loosen,
    /// the halves miscompute, or the line stops citing.
    #[test]
    fn working_fires_on_a_rising_catalog_shape() {
        let now = test_now();
        let rising = [0.5, 0.5, 0.5, 0.55, 0.75, 0.8, 0.8, 0.85];
        let out = practice_suggestions(&catalog_log(&rising, 1), &BTreeMap::new(), now);
        assert_eq!(out.len(), 1, "exactly the Working line: {out:?}");
        assert_eq!(out[0].kind, SuggestionKind::Working);
        assert!(
            out[0].text.contains("major up-down")
                && out[0].text.contains("51%")
                && out[0].text.contains("80%")
                && out[0].text.contains("8 graded"),
            "text embeds shape, halves, count: {}",
            out[0].text
        );
        assert!(
            out[0].evidence.contains("major up-down") && out[0].evidence.contains("2026-06-30"),
            "evidence cites the shape and newest date: {}",
            out[0].evidence
        );
    }

    /// AC2: each Working bar individually silences — below-K grades, a
    /// sub-0.15 delta, a stale newest grade, and ungraded rows never
    /// counting toward K. Fails if any bar loosens.
    #[test]
    fn working_needs_k_delta_and_recency() {
        let now = test_now();
        let no_mastery = BTreeMap::new();
        let rising = [0.5, 0.5, 0.5, 0.55, 0.75, 0.8, 0.8, 0.85];

        // 5 graded rows → below K.
        assert!(practice_suggestions(&catalog_log(&rising[3..], 1), &no_mastery, now).is_empty());

        // Delta 0.14 → below the bar; flat → nothing at all.
        let shy = [0.5, 0.5, 0.5, 0.5, 0.64, 0.64, 0.64, 0.64];
        assert!(practice_suggestions(&catalog_log(&shy, 1), &no_mastery, now).is_empty());
        assert!(practice_suggestions(&catalog_log(&[0.6; 8], 1), &no_mastery, now).is_empty());

        // Newest grade 15 days old → ancient history.
        assert!(practice_suggestions(&catalog_log(&rising, 15), &no_mastery, now).is_empty());

        // 5 graded rising + 3 ungraded deals = 8 rows but K counts GRADES —
        // and 5-of-8 graded is no bailing either. Total silence.
        let mut mixed = catalog_log(&rising[3..], 1);
        mixed.extend((0..3).map(|i| timed(&scale_spec(), 0, None, i)));
        assert!(practice_suggestions(&mixed, &no_mastery, now).is_empty());

        // NaN grades (spec §6) feed no claim: the delta comparison is false.
        assert!(practice_suggestions(&catalog_log(&[f64::NAN; 8], 1), &no_mastery, now).is_empty());
    }

    /// AC1's inclusive boundaries, pinned so the bars can't drift with
    /// green CI: EXACTLY 6 graded rows, a delta of EXACTLY +0.15 (0.25 and
    /// 0.4 are chosen so the f32 half-means and their difference are
    /// bit-exact), and a newest grade EXACTLY 14 days old still fire.
    /// Fails if K rises, `>=` becomes `>`, or the recency window shrinks.
    #[test]
    fn working_bars_are_inclusive() {
        let now = test_now();
        let no_mastery = BTreeMap::new();

        // 6 graded, delta == 0.15 exactly → still evidence.
        let at_the_bar = [0.25, 0.25, 0.25, 0.4, 0.4, 0.4];
        let out = practice_suggestions(&catalog_log(&at_the_bar, 1), &no_mastery, now);
        assert_eq!(out.len(), 1, "6 grades at a +0.15 delta fire: {out:?}");
        assert_eq!(out[0].kind, SuggestionKind::Working);
        assert!(
            out[0].text.contains("6 graded"),
            "the count is the window's: {}",
            out[0].text
        );

        // Newest grade 14 days old → still recent. (The older rows sit
        // 15–21 days back, so no neglect contrast exists and the Working
        // line is the only voice.)
        let rising = [0.5, 0.5, 0.5, 0.55, 0.75, 0.8, 0.8, 0.85];
        let aged = practice_suggestions(&catalog_log(&rising, 14), &no_mastery, now);
        assert_eq!(aged.len(), 1, "14 days is still evidence: {aged:?}");
        assert_eq!(aged[0].kind, SuggestionKind::Working);
    }

    /// AC3: the 12-row window — ancient lows before a recent plateau must
    /// not fake a climb (over the lifetime the halves would read a false
    /// rise). The tonic-0 20-day log legitimately earns neglect, so pin the
    /// absence of WORKING only. Fails if the window leaks lifetime rows.
    #[test]
    fn working_window_excludes_lifetime() {
        let now = test_now();
        let mut plateau = vec![0.2; 8];
        plateau.extend(vec![0.7; 12]);
        assert!(
            practice_suggestions(&catalog_log(&plateau, 1), &BTreeMap::new(), now)
                .iter()
                .all(|s| s.kind != SuggestionKind::Working),
            "ancient lows before a recent plateau must not read as working"
        );
    }

    /// AC4: the catalog gate. A rising player CELL is Momentum's claim, not
    /// Working's; a lifted PROGRESSION is player material too (S2's catalog
    /// dealer couldn't re-deal it); score-reference and junk rows earn no
    /// call at all. Fails if the gate drops and one history speaks with two
    /// voices — or claims material it can't deal.
    #[test]
    fn cells_scores_and_junk_never_earn_working() {
        let now = test_now();
        let no_mastery = BTreeMap::new();
        let rising = [0.5, 0.5, 0.5, 0.55, 0.75, 0.8, 0.8, 0.85];

        let cells: Vec<TimedExerciseLogEntry> = rising
            .iter()
            .enumerate()
            .map(|(i, &a)| timed(&cell_spec(), 0, Some(a), (rising.len() - 1 - i) as i64))
            .collect();
        let out = practice_suggestions(&cells, &no_mastery, now);
        assert!(
            out.iter().any(|s| s.kind == SuggestionKind::Momentum)
                && out.iter().all(|s| s.kind != SuggestionKind::Working),
            "a rising cell speaks once, as Momentum: {out:?}"
        );

        let scores: Vec<TimedExerciseLogEntry> = rising
            .iter()
            .enumerate()
            .map(|(i, &a)| {
                timed(
                    r#"{"score_title":"Autumn Leaves"}"#,
                    0,
                    Some(a),
                    (rising.len() - 1 - i) as i64,
                )
            })
            .collect();
        assert!(
            shape_verdicts(&scores, now).is_empty(),
            "scores earn no call"
        );

        let junk: Vec<TimedExerciseLogEntry> = rising
            .iter()
            .enumerate()
            .map(|(i, &a)| timed("{not json", 0, Some(a), (rising.len() - 1 - i) as i64))
            .collect();
        assert!(shape_verdicts(&junk, now).is_empty(), "junk earns no call");

        // A lifted progression, played to rising grades → no verdict.
        let mut prog_spec: variations::VariationSpec = serde_json::from_str(&scale_spec()).unwrap();
        prog_spec.scale = None;
        prog_spec.progression = Some(vec![variations::ProgressionStep {
            offset: 0,
            chord: variations::ChordType::Minor7,
        }]);
        let prog_json = serde_json::to_string(&prog_spec).unwrap();
        let progs: Vec<TimedExerciseLogEntry> = rising
            .iter()
            .enumerate()
            .map(|(i, &a)| timed(&prog_json, 0, Some(a), (rising.len() - 1 - i) as i64))
            .collect();
        assert!(
            shape_verdicts(&progs, now).is_empty(),
            "a lifted progression is the player's material, not the catalog's"
        );
    }

    /// AC5: Bailing — dealt ≥6 in the window, graded×3 ≤ dealt, recent →
    /// the verdict carries both counts; one fewer deal, a 3-of-6 grade
    /// ratio, or a stale window each silence it; and no shape ever carries
    /// both calls. Bailing never becomes a recap line. Fails if a bar
    /// loosens or the exclusivity breaks.
    #[test]
    fn bailing_fires_on_dealt_and_abandoned() {
        let now = test_now();
        // Rows oldest → newest, like the store returns them (ORDER BY id);
        // the grades land on the OLDEST rows so recent deals read as bails.
        let deals = |dealt: usize, graded: usize, newest_days: i64| -> Vec<TimedExerciseLogEntry> {
            (0..dealt)
                .map(|i| {
                    let acc = (i < graded).then_some(0.5);
                    timed(&scale_spec(), 0, acc, newest_days + (dealt - 1 - i) as i64)
                })
                .collect()
        };

        let out = shape_verdicts(&deals(8, 2, 1), now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].shape, "major up-down");
        assert_eq!(
            out[0].call,
            ShapeCall::Bailing {
                dealt: 8,
                graded: 2
            }
        );
        // …and it stays out of the recap.
        assert!(practice_suggestions(&deals(8, 2, 1), &BTreeMap::new(), now)
            .iter()
            .all(|s| s.kind != SuggestionKind::Working));

        // Boundary: 6 deals / 2 grades fires; 6 deals / 3 grades doesn't;
        // and at the full window the inclusive bar is 4-of-12 (12 ≤ 12) —
        // a fifth grade silences it.
        assert_eq!(shape_verdicts(&deals(6, 2, 1), now).len(), 1);
        assert!(shape_verdicts(&deals(6, 3, 1), now).is_empty());
        assert_eq!(shape_verdicts(&deals(12, 4, 1), now).len(), 1);
        assert!(shape_verdicts(&deals(12, 5, 1), now).is_empty());
        // Below the deal bar.
        assert!(shape_verdicts(&deals(5, 0, 1), now).is_empty());
        // Stale window.
        assert!(shape_verdicts(&deals(8, 0, 15), now).is_empty());

        // Recency reads the NEWEST row: a window whose oldest deals sit 22
        // days back still fires while its last deal is a day old. Fails if
        // the recency bar ever checks the window's oldest stamp.
        let spread: Vec<TimedExerciseLogEntry> = (0..8)
            .map(|i| timed(&scale_spec(), 0, None, 22 - i * 3))
            .collect();
        let out = shape_verdicts(&spread, now);
        assert_eq!(out.len(), 1, "old window, fresh bail still calls: {out:?}");
        assert!(matches!(out[0].call, ShapeCall::Bailing { .. }));

        // Exclusivity: a rising fully-graded shape is Working, never also
        // Bailing — one shape, one call.
        let rising = [0.5, 0.5, 0.5, 0.55, 0.75, 0.8, 0.8, 0.85];
        let verdicts = shape_verdicts(&catalog_log(&rising, 1), now);
        assert_eq!(verdicts.len(), 1);
        assert!(matches!(verdicts[0].call, ShapeCall::Working { .. }));
    }
}
