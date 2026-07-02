//! Learner Model — the evolving per-user aggregate every practice feature
//! compounds into (epic #252, foundation F2).
//!
//! This is the "sessions get smarter" spine: one versioned, additive blob that
//! features read and write through **pure, deterministic transitions** (no I/O,
//! no wall clock — time is injected), so every rule is unit-testable and a
//! given input always produces the same model.
//!
//! Today the blob carries the **collection** (unlocked reveals, #253 S3),
//! **per-key mastery** and the **adaptive difficulty step** (both for the
//! guided coach, #254). Streaks and the sound profile land in later slices —
//! the blob is forward-compatible by construction (`version` field + unknown
//! fields at both the top level and inside entries are preserved on a
//! read→write roundtrip), so those additions need no migration of stored rows.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Current schema version of the [`LearnerModel`] blob.
pub const LEARNER_MODEL_VERSION: u8 = 1;

/// The top of the adaptive difficulty ladder (steps are `0..=MAX_DIFFICULTY`).
/// The guided coach (#254) moves at most one bounded step per drill.
pub const MAX_DIFFICULTY: u8 = 9;

/// EWMA smoothing for per-key accuracy: high enough to adapt within a few
/// drills, low enough that one bad run doesn't erase a history of good ones.
pub const MASTERY_EWMA_ALPHA: f32 = 0.3;

/// A key/scale is **owned** at or above this smoothed accuracy…
pub const OWNED_ACCURACY_THRESHOLD: f32 = 0.85;

/// …provided it has been attempted at least this many times (one lucky run
/// can't claim ownership).
pub const OWNED_MIN_ATTEMPTS: u32 = 3;

/// One unlocked reveal: a musical concept tied to a real-world connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collected {
    /// The musical concept, e.g. `"G Dorian"`.
    pub concept: String,
    /// The grounded real-world connection, e.g. `"Miles Davis — \"So What\""`.
    pub connection: String,
    /// When this entry was first unlocked (Unix seconds, injected by the caller).
    pub first_seen_epoch_secs: i64,
    /// How many times this exact reveal has surfaced (1 on first unlock).
    pub count: u32,
    /// Forward-compatibility, same contract as [`LearnerModel::extra`]: per-entry
    /// fields added by a newer build survive an older build's read→write
    /// roundtrip instead of being silently stripped.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Per-key/scale mastery, updated by [`apply_drill_result`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mastery {
    /// Drills attempted for this key/scale.
    pub attempts: u32,
    /// Smoothed accuracy (0..1), EWMA with [`MASTERY_EWMA_ALPHA`].
    pub accuracy_ewma: f32,
    /// `true` while the smoothed accuracy holds [`OWNED_ACCURACY_THRESHOLD`]
    /// over ≥ [`OWNED_MIN_ATTEMPTS`] attempts. **Not sticky** — slipping back
    /// under the bar honestly loses the key (the wheel dims), it doesn't stay
    /// lit on past glory.
    pub owned: bool,
    /// Last attempt time (Unix seconds, injected).
    pub last_epoch_secs: i64,
    /// Forward-compatibility, same contract as [`LearnerModel::extra`].
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One drill's outcome, as the coach reports it (#254): which key/scale it
/// trained and the 0..1 accuracy achieved against the drill's exact target.
///
/// Carries the raw `(tonic, mode)` rather than a pre-formatted key so the
/// transition itself normalizes via [`mastery_key`] — a caller can't split one
/// key into two entries with `"Dorian"` vs `"dorian"` or tonic 19 vs 7.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillResult {
    /// Tonic pitch class (any octave; wrapped mod 12).
    pub tonic: u8,
    /// Mode/scale label (any casing; normalized).
    pub mode: String,
    /// 0..1 — correct notes / target notes.
    pub accuracy: f32,
}

/// The per-user learner model. Stored as one JSON blob (SQLite locally, a
/// nullable JSONB column in Supabase later) — additive and versioned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnerModel {
    /// Blob schema version — bump when a transition's meaning changes.
    pub version: u8,
    /// Unlocked reveals, keyed by [`collection_key`] for stable dedup.
    #[serde(default)]
    pub collection: BTreeMap<String, Collected>,
    /// Current adaptive difficulty step, `0..=MAX_DIFFICULTY`. Written by the
    /// guided coach at lesson end; read to start the next lesson where the
    /// player left off. Additive: blobs written before this field existed load
    /// as step 0 (gentlest).
    #[serde(default)]
    pub difficulty: u8,
    /// Per-key/scale mastery, keyed by [`mastery_key`]. Additive like
    /// `difficulty`.
    #[serde(default)]
    pub key_mastery: BTreeMap<String, Mastery>,
    /// Last transition time (Unix seconds, injected).
    pub updated_at_epoch_secs: i64,
    /// Forward-compatibility: top-level fields this build doesn't know yet
    /// (added by a newer version) are preserved across a read→write roundtrip
    /// instead of being silently dropped.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for LearnerModel {
    fn default() -> Self {
        Self {
            version: LEARNER_MODEL_VERSION,
            collection: BTreeMap::new(),
            difficulty: 0,
            key_mastery: BTreeMap::new(),
            updated_at_epoch_secs: 0,
            extra: serde_json::Map::new(),
        }
    }
}

impl LearnerModel {
    /// Number of distinct reveals unlocked.
    pub fn collection_size(&self) -> usize {
        self.collection.len()
    }
}

/// Stable dedup key for a collected reveal: same concept + same connection is
/// the same entry, regardless of how its `why` line was worded that day.
pub fn collection_key(concept: &str, connection: &str) -> String {
    format!("{}\u{1f}{}", concept.trim(), connection.trim())
}

/// Stable mastery key for a key/scale, e.g. tonic 7 + `"Dorian"` → `"7:dorian"`.
/// Tonic is the pitch class (0–11); the mode is normalized to lowercase so
/// perception's capitalized labels and the coach's specs can't split one key
/// into two entries.
pub fn mastery_key(tonic: u8, mode: &str) -> String {
    format!("{}:{}", tonic % 12, mode.trim().to_lowercase())
}

/// Pure transition: fold one drill result into per-key mastery (#252 F2, #254).
///
/// `attempts` increments; `accuracy_ewma` moves by [`MASTERY_EWMA_ALPHA`]
/// toward the new accuracy (a first attempt seeds the EWMA directly);
/// `owned` is recomputed from the smoothed value — it flips on at
/// [`OWNED_ACCURACY_THRESHOLD`] over ≥ [`OWNED_MIN_ATTEMPTS`] attempts and
/// honestly flips back off if the player slips. Deterministic; accuracy is
/// clamped to 0..=1 (a NaN is treated as 0).
pub fn apply_drill_result(
    model: &LearnerModel,
    result: &DrillResult,
    now_epoch_secs: i64,
) -> LearnerModel {
    let mut next = model.clone();
    let accuracy = if result.accuracy.is_nan() {
        0.0
    } else {
        result.accuracy.clamp(0.0, 1.0)
    };
    let entry = next
        .key_mastery
        .entry(mastery_key(result.tonic, &result.mode))
        .or_insert_with(|| Mastery {
            attempts: 0,
            accuracy_ewma: 0.0,
            owned: false,
            last_epoch_secs: now_epoch_secs,
            extra: serde_json::Map::new(),
        });
    // Self-heal a poisoned EWMA (e.g. a NaN smuggled in via a synced blob):
    // NaN would otherwise propagate forever and make the key un-ownable.
    if entry.accuracy_ewma.is_nan() {
        entry.accuracy_ewma = 0.0;
        entry.attempts = 0;
    }
    entry.accuracy_ewma = if entry.attempts == 0 {
        accuracy
    } else {
        entry.accuracy_ewma + MASTERY_EWMA_ALPHA * (accuracy - entry.accuracy_ewma)
    };
    entry.attempts = entry.attempts.saturating_add(1);
    entry.last_epoch_secs = now_epoch_secs;
    entry.owned =
        entry.attempts >= OWNED_MIN_ATTEMPTS && entry.accuracy_ewma >= OWNED_ACCURACY_THRESHOLD;
    next.updated_at_epoch_secs = now_epoch_secs;
    next
}

/// Pure transition: set the adaptive difficulty (clamped to `0..=MAX_DIFFICULTY`).
/// Written by the coach at lesson end (#254).
pub fn apply_difficulty(model: &LearnerModel, difficulty: u8, now_epoch_secs: i64) -> LearnerModel {
    let mut next = model.clone();
    next.difficulty = difficulty.min(MAX_DIFFICULTY);
    next.updated_at_epoch_secs = now_epoch_secs;
    next
}

/// Pure transition: fold one surfaced reveal into the model (#253 S3).
///
/// A **novel** `(concept, connection)` adds exactly one entry (`count = 1`,
/// `first_seen` = `now`); a **repeat** leaves the collection size unchanged and
/// only bumps that entry's `count` (its `first_seen` is preserved).
/// Deterministic: same inputs → same output.
pub fn apply_reveal(
    model: &LearnerModel,
    concept: &str,
    connection: &str,
    now_epoch_secs: i64,
) -> LearnerModel {
    let mut next = model.clone();
    let key = collection_key(concept, connection);
    next.collection
        .entry(key)
        .and_modify(|c| c.count = c.count.saturating_add(1))
        .or_insert_with(|| Collected {
            concept: concept.trim().to_owned(),
            connection: connection.trim().to_owned(),
            first_seen_epoch_secs: now_epoch_secs,
            count: 1,
            extra: serde_json::Map::new(),
        });
    next.updated_at_epoch_secs = now_epoch_secs;
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #253 S3 AC: a novel reveal grows the collection by exactly 1 with
    /// count = 1 and the injected timestamp. Fails if dedup keys collide or the
    /// entry isn't recorded.
    #[test]
    fn novel_reveal_adds_exactly_one_entry() {
        let m0 = LearnerModel::default();
        let m1 = apply_reveal(&m0, "G Dorian", "Miles Davis — \"So What\"", 100);
        assert_eq!(m1.collection_size(), 1);
        let entry = m1.collection.values().next().unwrap();
        assert_eq!(entry.count, 1);
        assert_eq!(entry.first_seen_epoch_secs, 100);
        assert_eq!(m1.updated_at_epoch_secs, 100);
        // The input model is untouched (pure transition).
        assert_eq!(m0.collection_size(), 0);
    }

    /// #253 S3 AC: a repeat of the same (concept, connection) grows the
    /// collection by exactly 0 — only the count bumps; first_seen is preserved.
    /// Fails if dedup breaks (a duplicate entry would appear).
    #[test]
    fn repeat_reveal_does_not_grow_the_collection() {
        let m0 = LearnerModel::default();
        let m1 = apply_reveal(&m0, "G Dorian", "Santana — \"Oye Como Va\"", 100);
        let m2 = apply_reveal(&m1, "G Dorian", "Santana — \"Oye Como Va\"", 200);
        assert_eq!(m2.collection_size(), 1, "repeat must not add an entry");
        let entry = m2.collection.values().next().unwrap();
        assert_eq!(entry.count, 2);
        assert_eq!(entry.first_seen_epoch_secs, 100, "first_seen preserved");
        assert_eq!(m2.updated_at_epoch_secs, 200);
    }

    /// Same concept with a *different* connection is a different unlock.
    #[test]
    fn different_connection_is_a_new_entry() {
        let m0 = LearnerModel::default();
        let m1 = apply_reveal(&m0, "G Dorian", "Miles Davis — \"So What\"", 1);
        let m2 = apply_reveal(&m1, "G Dorian", "Santana — \"Oye Como Va\"", 2);
        assert_eq!(m2.collection_size(), 2);
    }

    /// Deterministic: identical inputs produce identical models.
    #[test]
    fn transition_is_deterministic() {
        let m0 = LearnerModel::default();
        assert_eq!(
            apply_reveal(&m0, "C Major", "Beethoven — \"Ode to Joy\"", 42),
            apply_reveal(&m0, "C Major", "Beethoven — \"Ode to Joy\"", 42),
        );
    }

    /// Forward-compatibility: unknown fields written by a newer build — both
    /// **top-level** and **per-collection-entry** — survive a read → transition
    /// → write roundtrip instead of being dropped. Fails if either flatten is
    /// removed: a newer build's data would then be silently destroyed by an
    /// older one.
    #[test]
    fn roundtrip_preserves_unknown_fields() {
        let json = r#"{
            "version": 2,
            "collection": {
                "C Major\u001FBeethoven": {
                    "concept": "C Major",
                    "connection": "Beethoven",
                    "first_seen_epoch_secs": 1,
                    "count": 2,
                    "mastery_note": "from-v2"
                }
            },
            "key_mastery": {
                "7:dorian": {
                    "attempts": 2,
                    "accuracy_ewma": 0.8,
                    "owned": false,
                    "last_epoch_secs": 3,
                    "streak_in_key": 5
                }
            },
            "updated_at_epoch_secs": 5,
            "sound_profile": { "mode_lean": "minor" },
            "streak": { "count": 7 }
        }"#;
        let model: LearnerModel = serde_json::from_str(json).expect("newer blob parses");
        assert_eq!(model.version, 2);
        let after = apply_reveal(&model, "G Dorian", "Miles Davis", 6);
        // Drill the SAME mastery key so the per-Mastery unknown field survives
        // the entry-update path, not just an untouched clone.
        let after = apply_drill_result(
            &after,
            &DrillResult {
                tonic: 7,
                mode: "Dorian".to_owned(),
                accuracy: 1.0,
            },
            7,
        );
        let out = serde_json::to_value(&after).expect("serializes");
        // Top-level unknown fields preserved.
        assert_eq!(out["sound_profile"]["mode_lean"], "minor");
        assert_eq!(out["streak"]["count"], 7);
        // Per-entry unknown fields preserved too — including through the
        // count-bump path of an existing entry.
        assert_eq!(
            out["collection"]["C Major\u{1f}Beethoven"]["mastery_note"],
            "from-v2"
        );
        // Per-Mastery unknown fields preserved through apply_drill_result.
        assert_eq!(out["key_mastery"]["7:dorian"]["streak_in_key"], 5);
        assert_eq!(after.key_mastery["7:dorian"].attempts, 3);
        assert_eq!(after.collection_size(), 2);
    }

    /// The dedup separator (`\u{1f}`) is an internal contract: concepts and
    /// connections come from the curated reveal path and never contain it, so
    /// distinct pairs can't alias. This pins that two pairs sharing characters
    /// around the separator still produce distinct keys in practice.
    #[test]
    fn collection_keys_do_not_alias_for_real_inputs() {
        assert_ne!(
            collection_key("G Dorian", "Miles Davis"),
            collection_key("G", "Dorian Miles Davis"),
        );
    }

    /// #254/F2: `owned` flips on only at the threshold over enough attempts —
    /// two perfect drills aren't ownership (min attempts), three are; and the
    /// EWMA means one bad run after that dents the average but the flag flips
    /// back off only when the *smoothed* value slips under the bar (honest,
    /// non-sticky). Fails if the flip rule or EWMA math regresses.
    #[test]
    fn owned_flips_at_threshold_over_min_attempts_and_is_not_sticky() {
        let key = mastery_key(7, "Dorian");
        let r = |a: f32| DrillResult {
            tonic: 7,
            mode: "Dorian".to_owned(),
            accuracy: a,
        };
        let m0 = LearnerModel::default();
        let m1 = apply_drill_result(&m0, &r(1.0), 1);
        let m2 = apply_drill_result(&m1, &r(1.0), 2);
        assert!(
            !m2.key_mastery[&key].owned,
            "2 attempts must not own (min is {OWNED_MIN_ATTEMPTS})"
        );
        let m3 = apply_drill_result(&m2, &r(1.0), 3);
        assert!(m3.key_mastery[&key].owned, "3 perfect drills own the key");

        // Slip hard: repeated zeros drag the EWMA under the bar → honestly lost.
        let mut m = m3;
        for t in 4..12 {
            m = apply_drill_result(&m, &r(0.0), t);
        }
        assert!(
            !m.key_mastery[&key].owned,
            "sustained failure must un-own (ewma {})",
            m.key_mastery[&key].accuracy_ewma
        );
    }

    /// EWMA math: a first attempt seeds directly; later attempts move by alpha.
    /// Fails on an off-by-one in the seeding or a wrong smoothing direction.
    #[test]
    fn ewma_seeds_then_smooths() {
        let key = mastery_key(0, "major");
        let m0 = LearnerModel::default();
        let m1 = apply_drill_result(
            &m0,
            &DrillResult {
                tonic: 0,
                mode: "major".to_owned(),
                accuracy: 0.6,
            },
            1,
        );
        assert!((m1.key_mastery[&key].accuracy_ewma - 0.6).abs() < 1e-6);
        let m2 = apply_drill_result(
            &m1,
            &DrillResult {
                tonic: 0,
                mode: "major".to_owned(),
                accuracy: 1.0,
            },
            2,
        );
        // 0.6 + 0.3 * (1.0 - 0.6) = 0.72
        assert!((m2.key_mastery[&key].accuracy_ewma - 0.72).abs() < 1e-6);
        assert_eq!(m2.key_mastery[&key].attempts, 2);
    }

    /// Garbage accuracy can't corrupt the model: NaN counts as 0, out-of-range
    /// values clamp.
    #[test]
    fn accuracy_is_sanitized() {
        let key = mastery_key(2, "minor");
        let m0 = LearnerModel::default();
        let m1 = apply_drill_result(
            &m0,
            &DrillResult {
                tonic: 2,
                mode: "minor".to_owned(),
                accuracy: f32::NAN,
            },
            1,
        );
        assert_eq!(m1.key_mastery[&key].accuracy_ewma, 0.0);
        let m2 = apply_drill_result(
            &m1,
            &DrillResult {
                tonic: 2,
                mode: "minor".to_owned(),
                accuracy: 7.0,
            },
            2,
        );
        assert!(m2.key_mastery[&key].accuracy_ewma <= 1.0);
    }

    /// `mastery_key` normalizes: perception's "Dorian" and the coach's "dorian"
    /// are one key; tonic wraps into a pitch class.
    #[test]
    fn mastery_key_normalizes_mode_and_tonic() {
        assert_eq!(mastery_key(7, "Dorian"), mastery_key(19, " dorian "));
    }

    /// Normalization is structural, not opt-in: two results naming the same key
    /// differently (case, whitespace, octave-offset tonic) land in ONE entry.
    /// Fails if apply_drill_result stops routing through mastery_key — split
    /// entries would corrupt attempts/EWMA and double-count on the wheel.
    #[test]
    fn equivalent_drill_results_share_one_entry() {
        let m0 = LearnerModel::default();
        let m1 = apply_drill_result(
            &m0,
            &DrillResult {
                tonic: 7,
                mode: "Dorian".to_owned(),
                accuracy: 1.0,
            },
            1,
        );
        let m2 = apply_drill_result(
            &m1,
            &DrillResult {
                tonic: 19,
                mode: " dorian ".to_owned(),
                accuracy: 1.0,
            },
            2,
        );
        assert_eq!(m2.key_mastery.len(), 1, "equivalent keys must not split");
        assert_eq!(m2.key_mastery[&mastery_key(7, "dorian")].attempts, 2);
    }

    /// Drilling one key leaves every other key's mastery untouched.
    #[test]
    fn drilling_one_key_does_not_touch_another() {
        let m0 = LearnerModel::default();
        let m1 = apply_drill_result(
            &m0,
            &DrillResult {
                tonic: 0,
                mode: "major".to_owned(),
                accuracy: 0.9,
            },
            1,
        );
        let before = m1.key_mastery[&mastery_key(0, "major")].clone();
        let m2 = apply_drill_result(
            &m1,
            &DrillResult {
                tonic: 7,
                mode: "dorian".to_owned(),
                accuracy: 0.2,
            },
            2,
        );
        assert_eq!(m2.key_mastery[&mastery_key(0, "major")], before);
        assert_eq!(m2.key_mastery.len(), 2);
    }

    /// Difficulty transition clamps to the ladder and stamps the time.
    #[test]
    fn apply_difficulty_clamps_to_the_ladder() {
        let m = apply_difficulty(&LearnerModel::default(), MAX_DIFFICULTY + 5, 9);
        assert_eq!(m.difficulty, MAX_DIFFICULTY);
        assert_eq!(m.updated_at_epoch_secs, 9);
    }

    /// A pre-mastery blob (no difficulty/key_mastery fields at all) still loads,
    /// defaulting to step 0 and an empty map — the additive-schema contract.
    #[test]
    fn old_blob_without_mastery_fields_loads_with_defaults() {
        let json = r#"{ "version": 1, "collection": {}, "updated_at_epoch_secs": 4 }"#;
        let model: LearnerModel = serde_json::from_str(json).expect("old blob parses");
        assert_eq!(model.difficulty, 0);
        assert!(model.key_mastery.is_empty());
    }
}
