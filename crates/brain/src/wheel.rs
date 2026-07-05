//! The 12-key mastery wheel (#256): a pure, read-only view over the Learner
//! Model (F2) + stored fingerprint history. All derivation lives here so the
//! rules are unit-testable and the wheel can never drift from F2's own
//! owned-flip thresholds (the constants are imported, not redefined).

use serde::{Deserialize, Serialize};

use crate::fingerprint::MusicalFingerprint;
use crate::learner::{LearnerModel, Mastery, OWNED_ACCURACY_THRESHOLD, OWNED_MIN_ATTEMPTS};

/// Mastery state for one chromatic root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    None,
    Learning,
    Owned,
}

/// One wheel cell: everything the UI shows for a chromatic root, aggregated
/// across every scale/mode practiced on that root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyCell {
    /// Pitch class 0–11 (C = 0).
    pub tonic: u8,
    pub state: KeyState,
    /// Total drills across all this root's mastery entries.
    pub attempts: u32,
    /// Best smoothed accuracy among this root's entries (0..1).
    pub best_accuracy: f32,
    /// Scale/mode names practiced on this root (learning or owned), sorted.
    pub scales: Vec<String>,
}

/// Direction of a measured dimension over recent sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    Improving,
    Steady,
    Slipping,
    /// Too few qualifying sessions to say anything honest.
    Unknown,
}

/// The whole wheel, one snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WheelView {
    /// One cell per chromatic root, indexed by pitch class (the view lays them
    /// out in circle-of-fifths order).
    pub cells: Vec<KeyCell>,
    pub intonation_trend: Trend,
    pub tone_trend: Trend,
    pub total_owned: u8,
}

/// The single classification rule — must agree with F2's owned flip exactly,
/// so it re-checks the same imported thresholds rather than trusting `owned`
/// blindly (a stale/synced blob can't show a glowing key that F2's own rule
/// would reject).
pub fn classify(m: &Mastery) -> KeyState {
    if m.owned && m.attempts >= OWNED_MIN_ATTEMPTS && m.accuracy_ewma >= OWNED_ACCURACY_THRESHOLD {
        KeyState::Owned
    } else if m.attempts >= 1 {
        KeyState::Learning
    } else {
        KeyState::None
    }
}

/// Fewer qualifying fingerprints than this ⇒ [`Trend::Unknown`].
const MIN_TREND_POINTS: usize = 4;
/// Half-to-half differences inside this band read as [`Trend::Steady`].
const TREND_EPS: f32 = 0.05;

/// Older-half vs newer-half comparison; `higher_is_better` orients the sign.
fn trend_of(values: &[f32], higher_is_better: bool) -> Trend {
    if values.len() < MIN_TREND_POINTS {
        return Trend::Unknown;
    }
    let mid = values.len() / 2;
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let delta = mean(&values[mid..]) - mean(&values[..mid]);
    let signed = if higher_is_better { delta } else { -delta };
    // Normalize the epsilon to the older half's scale so cents (tens) and
    // clarity (0..1) use one rule.
    let scale = mean(&values[..mid]).abs().max(1e-3);
    if signed / scale > TREND_EPS {
        Trend::Improving
    } else if signed / scale < -TREND_EPS {
        Trend::Slipping
    } else {
        Trend::Steady
    }
}

/// Build the wheel. Pure and deterministic; `fingerprints` must be ordered
/// **oldest → newest** (the trends compare the halves).
pub fn build_wheel(model: &LearnerModel, fingerprints: &[MusicalFingerprint]) -> WheelView {
    let mut cells: Vec<KeyCell> = (0..12u8)
        .map(|tonic| KeyCell {
            tonic,
            state: KeyState::None,
            attempts: 0,
            best_accuracy: 0.0,
            scales: Vec::new(),
        })
        .collect();

    for (key, mastery) in &model.key_mastery {
        // Mastery keys are "tonic:mode" (learner::mastery_key).
        let Some((tonic_str, mode)) = key.split_once(':') else {
            continue;
        };
        let Ok(tonic) = tonic_str.parse::<u8>() else {
            continue;
        };
        let cell = &mut cells[usize::from(tonic % 12)];
        cell.attempts = cell.attempts.saturating_add(mastery.attempts);
        if mastery.accuracy_ewma > cell.best_accuracy {
            cell.best_accuracy = mastery.accuracy_ewma;
        }
        // The root's state is the BEST state among its entries.
        let state = classify(mastery);
        if state == KeyState::Owned || (state == KeyState::Learning && cell.state == KeyState::None)
        {
            cell.state = state;
        }
        if mastery.attempts >= 1 && !mode.is_empty() {
            cell.scales.push(mode.to_owned());
        }
    }
    for cell in &mut cells {
        cell.scales.sort();
        cell.scales.dedup();
    }

    let intonation: Vec<f32> = fingerprints
        .iter()
        .filter_map(|f| f.intonation.as_ref().map(|i| i.mean_abs_cents))
        .collect();
    let tone: Vec<f32> = fingerprints
        .iter()
        .filter_map(|f| f.tone.as_ref().map(|t| t.core_clarity))
        .collect();

    let total_owned = cells.iter().filter(|c| c.state == KeyState::Owned).count() as u8;
    WheelView {
        cells,
        // Lower cents = better; higher clarity = better.
        intonation_trend: trend_of(&intonation, false),
        tone_trend: trend_of(&tone, true),
        total_owned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learner::{apply_drill_result, DrillResult};

    fn drilled(
        model: &LearnerModel,
        tonic: u8,
        mode: &str,
        acc: f32,
        times: usize,
    ) -> LearnerModel {
        let mut m = model.clone();
        for t in 0..times {
            m = apply_drill_result(
                &m,
                &DrillResult {
                    tonic,
                    mode: mode.to_owned(),
                    accuracy: acc,
                },
                t as i64,
            );
        }
        m
    }

    /// #256 AC: the wheel's classification agrees with F2's own owned flip —
    /// three high-accuracy drills own the key on both sides; two don't. Fails
    /// if the wheel redefines (and drifts from) F2's thresholds.
    #[test]
    fn classify_agrees_with_f2s_owned_flip() {
        let m2 = drilled(&LearnerModel::default(), 7, "dorian", 1.0, 2);
        let key = crate::learner::mastery_key(7, "dorian");
        assert_eq!(classify(&m2.key_mastery[&key]), KeyState::Learning);

        let m3 = drilled(&LearnerModel::default(), 7, "dorian", 1.0, 3);
        assert_eq!(classify(&m3.key_mastery[&key]), KeyState::Owned);
        assert_eq!(build_wheel(&m3, &[]).total_owned, 1);
    }

    /// A synced/tampered blob claiming `owned: true` below the thresholds is
    /// shown honestly as Learning — the wheel re-checks, never trusts the flag.
    #[test]
    fn wheel_rechecks_a_stale_owned_flag() {
        // Attempts leg: owned=true with too few attempts reads Learning.
        let mut m = drilled(&LearnerModel::default(), 0, "major", 0.5, 1);
        let key = crate::learner::mastery_key(0, "major");
        m.key_mastery.get_mut(&key).unwrap().owned = true; // lying flag
        assert_eq!(classify(&m.key_mastery[&key]), KeyState::Learning);

        // Accuracy leg: owned=true, ENOUGH attempts, but a tanked EWMA — the
        // wheel must still refuse the glow (a synced/tampered blob can't
        // fake mastery past either gate).
        let mut m2 = drilled(&LearnerModel::default(), 0, "major", 0.2, 3);
        let entry = m2.key_mastery.get_mut(&key).unwrap();
        entry.owned = true;
        assert!(entry.attempts >= 3);
        assert!(entry.accuracy_ewma < 0.5);
        assert_eq!(classify(&m2.key_mastery[&key]), KeyState::Learning);
    }

    /// Cells aggregate across a root's modes: state is the BEST entry, scales
    /// list every practiced mode, attempts sum.
    #[test]
    fn cells_aggregate_across_modes_per_root() {
        let m = drilled(&LearnerModel::default(), 7, "dorian", 1.0, 3); // owned
        let m = drilled(&m, 7, "major", 0.4, 2); // learning, same root
        let wheel = build_wheel(&m, &[]);
        let g = &wheel.cells[7];
        assert_eq!(g.state, KeyState::Owned, "best entry wins the cell state");
        assert_eq!(g.attempts, 5);
        assert_eq!(g.scales, vec!["dorian".to_owned(), "major".to_owned()]);
        assert!(g.best_accuracy > 0.9);
        // Untouched roots stay None with a defined empty shape.
        assert_eq!(wheel.cells[0].state, KeyState::None);
        assert!(wheel.cells[0].scales.is_empty());
    }

    /// #256 AC (first run): an empty model renders all 12 cells as None —
    /// a defined empty state, no panic.
    #[test]
    fn empty_model_is_a_defined_empty_wheel() {
        let wheel = build_wheel(&LearnerModel::default(), &[]);
        assert_eq!(wheel.cells.len(), 12);
        assert!(wheel.cells.iter().all(|c| c.state == KeyState::None));
        assert_eq!(wheel.total_owned, 0);
        assert_eq!(wheel.intonation_trend, Trend::Unknown);
    }

    /// Trends: intonation improves when cents SHRINK, tone when clarity GROWS;
    /// tiny drifts read Steady; too few points read Unknown. Fails on a sign
    /// flip or a dropped qualifying gate.
    #[test]
    fn trends_orient_correctly() {
        assert_eq!(trend_of(&[30.0, 28.0, 12.0, 10.0], false), Trend::Improving);
        assert_eq!(trend_of(&[10.0, 10.0, 30.0, 31.0], false), Trend::Slipping);
        assert_eq!(trend_of(&[0.4, 0.4, 0.7, 0.7], true), Trend::Improving);
        assert_eq!(
            trend_of(&[0.5, 0.5, 0.505, 0.505], true),
            Trend::Steady,
            "sub-epsilon drift is Steady"
        );
        assert_eq!(trend_of(&[1.0, 2.0, 3.0], true), Trend::Unknown);
    }
}
