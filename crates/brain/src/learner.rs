//! Learner Model — the evolving per-user aggregate every practice feature
//! compounds into (epic #252, foundation F2).
//!
//! This is the "sessions get smarter" spine: one versioned, additive blob that
//! features read and write through **pure, deterministic transitions** (no I/O,
//! no wall clock — time is injected), so every rule is unit-testable and a
//! given input always produces the same model.
//!
//! This first slice carries only the **collection** (real-world music reveals
//! the player has unlocked, #253 S3). Key mastery, difficulty, streaks, and the
//! sound profile land in later slices — the blob is forward-compatible by
//! construction (`version` field + unknown top-level fields are preserved on a
//! read→write roundtrip), so those additions need no migration of stored rows.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Current schema version of the [`LearnerModel`] blob.
pub const LEARNER_MODEL_VERSION: u8 = 1;

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

/// The per-user learner model. Stored as one JSON blob (SQLite locally, a
/// nullable JSONB column in Supabase later) — additive and versioned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnerModel {
    /// Blob schema version — bump when a transition's meaning changes.
    pub version: u8,
    /// Unlocked reveals, keyed by [`collection_key`] for stable dedup.
    #[serde(default)]
    pub collection: BTreeMap<String, Collected>,
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
            "updated_at_epoch_secs": 5,
            "key_mastery": { "G:dorian": { "attempts": 3 } },
            "streak": { "count": 7 }
        }"#;
        let model: LearnerModel = serde_json::from_str(json).expect("newer blob parses");
        assert_eq!(model.version, 2);
        let after = apply_reveal(&model, "G Dorian", "Miles Davis", 6);
        let out = serde_json::to_value(&after).expect("serializes");
        // Top-level unknown fields preserved.
        assert_eq!(out["key_mastery"]["G:dorian"]["attempts"], 3);
        assert_eq!(out["streak"]["count"], 7);
        // Per-entry unknown fields preserved too — including through the
        // count-bump path of an existing entry.
        assert_eq!(
            out["collection"]["C Major\u{1f}Beethoven"]["mastery_note"],
            "from-v2"
        );
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
}
