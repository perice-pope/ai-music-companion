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
//! guided coach, #254), the **sound profile** (#258), and the **daily streak**
//! (#257). The blob is forward-compatible by construction (`version` field +
//! unknown fields at both the top level and inside entries are preserved on a
//! read→write roundtrip), so additions need no migration of stored rows.

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
    /// The musical concept, e.g. `"Dorian"` (mode-level since #388; blobs
    /// written before that may still hold key-stamped legacy values like
    /// `"G Dorian"` — [`apply_reveal`] folds those onto the mode on its next
    /// transition).
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

/// A calendar day in the user's local timezone, expressed as the count of days
/// since the Unix epoch (1970-01-01) *in that timezone*. The instant → LocalDay
/// conversion happens exactly once, at the IPC/command boundary, using the OS
/// local offset (#257 S3). The pure core only ever sees this ordinal — so every
/// streak transition is deterministic and needs no clock and no tz database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LocalDay(pub i64);

/// The daily-warmup streak (#257): how many consecutive local days the warmup
/// has been completed, and the last day that counted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Streak {
    /// Consecutive completed days. `0` until the first-ever completion.
    #[serde(default)]
    pub count: u32,
    /// The most recent local day a completion counted toward the streak.
    /// `None` until the first-ever completion.
    #[serde(default)]
    pub last_completed_local_day: Option<LocalDay>,
    /// Forward-compatibility, same contract as [`LearnerModel::extra`].
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Streak {
    fn default() -> Self {
        Self {
            count: 0,
            last_completed_local_day: None,
            extra: serde_json::Map::new(),
        }
    }
}

/// The most recent daily-warmup result (#257), read by the home surface and
/// by the same-day idempotency rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyWarmup {
    /// The day this result is for.
    pub day: LocalDay,
    /// Best score recorded for that day, clamped to `0.0..=1.0`.
    pub score: f32,
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
    /// The derived "your sound" snapshot (#258), written by
    /// [`crate::mirror::derive_sound_profile`] at read time. Additive; blobs
    /// from before this field existed load as `None`, and partial profiles
    /// from other versions parse via per-field defaults.
    #[serde(default)]
    pub sound_profile: Option<crate::mirror::SoundProfile>,
    /// Per-key/scale mastery, keyed by [`mastery_key`]. Additive like
    /// `difficulty`.
    #[serde(default)]
    pub key_mastery: BTreeMap<String, Mastery>,
    /// The daily-warmup streak (#257), advanced by [`apply_daily_completion`].
    /// Additive: blobs from before this field existed load as the zero streak.
    #[serde(default)]
    pub streak: Streak,
    /// The most recent daily-warmup result (#257). Additive; `None` until the
    /// first-ever completion.
    #[serde(default)]
    pub last_warmup: Option<DailyWarmup>,
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
            sound_profile: None,
            key_mastery: BTreeMap::new(),
            streak: Streak::default(),
            last_warmup: None,
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

/// Pure transition: apply a completed daily warmup for local day `day` with
/// grade `score` (#257 S1).
///
/// The streak measures **showing up, not performance**: any completion counts
/// regardless of `score` (even `0.0`) — the score only feeds `last_warmup`.
/// Exact math, with `prev = streak.last_completed_local_day`:
/// - **first ever** (`prev == None`): count = 1, today recorded.
/// - **same day** (`day == prev`): idempotent — count and `prev` unchanged;
///   only the recorded daily score rises to the best attempt of the day.
/// - **consecutive** (`day == prev + 1`): count saturates up by 1.
/// - **missed ≥ 1 full day** (`day > prev + 1`): count resets to 1 — today is
///   the first day of the new chain.
/// - **backward day** (`day < prev`, clock rollback / DST / out-of-order
///   replay): ignored entirely — the streak never decrements and `prev` never
///   moves backward.
///
/// Deterministic and clock-free: same `(model, day, score)` → identical
/// output; no I/O. `score` is clamped to `0.0..=1.0` (NaN counts as 0, same
/// sanitization as [`apply_drill_result`]). All other model fields are
/// preserved unchanged, including `updated_at_epoch_secs` — the signature
/// takes no wall time, and stamping is the callers' concern.
pub fn apply_daily_completion(m: &LearnerModel, day: LocalDay, score: f32) -> LearnerModel {
    let mut next = m.clone();
    let score = if score.is_nan() {
        0.0
    } else {
        score.clamp(0.0, 1.0)
    };
    let warmup = |s: f32| {
        Some(DailyWarmup {
            day,
            score: s,
            extra: serde_json::Map::new(),
        })
    };
    match m.streak.last_completed_local_day {
        None => {
            next.streak.count = 1;
            next.streak.last_completed_local_day = Some(day);
            next.last_warmup = warmup(score);
        }
        Some(last) if day.0 == last.0 => {
            // Same-day repeat: never advances the chain, only keeps the best
            // score of the day. A prior warmup from another day (possible in a
            // blob merged from sync) doesn't cap today's score.
            let prior = next
                .last_warmup
                .as_ref()
                .filter(|w| w.day == day)
                .map_or(0.0, |w| if w.score.is_nan() { 0.0 } else { w.score });
            next.last_warmup = warmup(score.max(prior));
        }
        Some(last) if day.0 == last.0 + 1 => {
            next.streak.count = next.streak.count.saturating_add(1);
            next.streak.last_completed_local_day = Some(day);
            next.last_warmup = warmup(score);
        }
        Some(last) if day.0 > last.0 => {
            // A full day (or more) skipped: today restarts the chain at 1.
            next.streak.count = 1;
            next.streak.last_completed_local_day = Some(day);
            next.last_warmup = warmup(score);
        }
        Some(_) => {} // Backward day: stale input, model untouched.
    }
    next
}

/// #388 legacy repair: pre-#388 reveals stored key-stamped concepts
/// (`"G Dorian"`) that fabricated a key claim the curated catalog never made.
/// Fold such a concept onto its mode-level form (`"Dorian"`) when — and only
/// when — it is `<sharp-side note> <curated mode display>`; anything else
/// passes through untouched. The note check uses the exact spelling set the
/// old `reveal_for` emitted, so no other concept shape can be mangled.
fn normalize_concept(concept: &str) -> String {
    const LEGACY_NOTES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let trimmed = concept.trim();
    if let Some((note, rest)) = trimmed.split_once(' ') {
        if LEGACY_NOTES.contains(&note) {
            if let Some((display, _)) = crate::connections::curated_for(&rest.trim().to_lowercase())
            {
                return display.to_owned();
            }
        }
    }
    trimmed.to_owned()
}

/// Pure transition: fold one surfaced reveal into the model (#253 S3).
///
/// A **novel** `(concept, connection)` adds exactly one entry (`count = 1`,
/// `first_seen` = `now`); a **repeat** leaves the collection size unchanged and
/// only bumps that entry's `count` (its `first_seen` is preserved).
/// Deterministic: same inputs → same output.
///
/// #388: concepts are mode-level. Both the incoming concept and any legacy
/// key-stamped entries already in the blob are folded through
/// [`normalize_concept`] first (merging counts, keeping the earliest
/// `first_seen`), so a post-#388 re-unlock dedups against its pre-#388 entry
/// instead of double-counting, and the fabricated key claim leaves the blob.
pub fn apply_reveal(
    model: &LearnerModel,
    concept: &str,
    connection: &str,
    now_epoch_secs: i64,
) -> LearnerModel {
    let mut next = model.clone();
    let legacy = std::mem::take(&mut next.collection);
    for (_, mut entry) in legacy {
        entry.concept = normalize_concept(&entry.concept);
        let key = collection_key(&entry.concept, &entry.connection);
        next.collection
            .entry(key)
            .and_modify(|c| {
                c.count = c.count.saturating_add(entry.count);
                c.first_seen_epoch_secs = c.first_seen_epoch_secs.min(entry.first_seen_epoch_secs);
            })
            .or_insert(entry);
    }
    let concept = normalize_concept(concept);
    let key = collection_key(&concept, connection);
    next.collection
        .entry(key)
        .and_modify(|c| c.count = c.count.saturating_add(1))
        .or_insert_with(|| Collected {
            concept,
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

    /// A pre-#388 blob with a key-stamped concept, as stored on disk.
    fn legacy_blob() -> LearnerModel {
        serde_json::from_str(
            r#"{
                "version": 1,
                "collection": {
                    "G Dorian\u001FMiles Davis — \"So What\"": {
                        "concept": "G Dorian",
                        "connection": "Miles Davis — \"So What\"",
                        "first_seen_epoch_secs": 100,
                        "count": 3
                    },
                    "A Dorian\u001FMiles Davis — \"So What\"": {
                        "concept": "A Dorian",
                        "connection": "Miles Davis — \"So What\"",
                        "first_seen_epoch_secs": 50,
                        "count": 1
                    }
                },
                "updated_at_epoch_secs": 100
            }"#,
        )
        .expect("legacy blob parses")
    }

    /// #388 (review MF2): a post-#388 mode-level reveal dedups against its
    /// pre-#388 key-stamped entry instead of double-counting — the legacy
    /// entries fold onto the mode (merging counts, keeping the earliest
    /// first_seen) and the incoming reveal is a repeat. Fails if a migrated
    /// DB's "N in your collection" bumps on re-unlocking an old reveal, or if
    /// the fabricated key claim ("G Dorian") survives in the blob.
    #[test]
    fn legacy_key_stamped_entries_merge_with_mode_level_repeat() {
        let m1 = apply_reveal(&legacy_blob(), "Dorian", "Miles Davis — \"So What\"", 200);
        assert_eq!(
            m1.collection_size(),
            1,
            "legacy G/A Dorian + the new Dorian are ONE unlock"
        );
        let entry = m1.collection.values().next().unwrap();
        assert_eq!(entry.concept, "Dorian", "the key claim leaves the blob");
        assert_eq!(entry.count, 5, "3 + 1 legacy counts + the new repeat");
        assert_eq!(
            entry.first_seen_epoch_secs, 50,
            "earliest legacy first_seen wins"
        );
    }

    /// #388: a legacy-shaped concept whose remainder is NOT a curated mode
    /// passes through normalization untouched — only the exact
    /// `<note> <curated mode>` shape the old reveal path produced is folded.
    /// Fails if normalization mangles concepts it can't prove are legacy.
    #[test]
    fn unknown_concept_shapes_survive_normalization_unchanged() {
        let m0 = LearnerModel::default();
        let m1 = apply_reveal(&m0, "G Mystery Sound", "Somebody", 1);
        let m2 = apply_reveal(&m1, "Dorian", "Miles Davis — \"So What\"", 2);
        assert_eq!(m2.collection_size(), 2);
        assert!(
            m2.collection
                .values()
                .any(|c| c.concept == "G Mystery Sound"),
            "non-mode concept must not be rewritten"
        );
    }

    /// #388: an old caller still passing a key-stamped concept can't
    /// reintroduce the fabrication — the input itself is normalized on write.
    #[test]
    fn key_stamped_input_is_stored_mode_level() {
        let m1 = apply_reveal(&LearnerModel::default(), "G# Major", "Beethoven", 1);
        let entry = m1.collection.values().next().unwrap();
        assert_eq!(entry.concept, "Major");
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
            "streak": { "count": 7, "freeze_tokens": 2 },
            "warmup_history": [ { "day": 3, "score": 0.5 } ]
        }"#;
        let model: LearnerModel = serde_json::from_str(json).expect("newer blob parses");
        assert_eq!(model.version, 2);
        // `streak` is a known field since #257 — it parses, and its own
        // unknown sub-fields ride the entry-level flatten.
        assert_eq!(model.streak.count, 7);
        assert_eq!(model.streak.last_completed_local_day, None);
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
        assert_eq!(out["warmup_history"][0]["score"], 0.5);
        // Known-field roundtrip + per-Streak unknown fields preserved.
        assert_eq!(out["streak"]["count"], 7);
        assert_eq!(out["streak"]["freeze_tokens"], 2);
        // Per-entry unknown fields preserved too — including through #388's
        // legacy-concept normalization, which re-keys "C Major" → "Major".
        assert_eq!(
            out["collection"]["Major\u{1f}Beethoven"]["mastery_note"],
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

    // ── #257 S1: streak transition ────────────────────────────────────

    /// A model mid-chain: `count` completed days ending on `last_day`.
    fn streaked(count: u32, last_day: i64) -> LearnerModel {
        let mut m = LearnerModel::default();
        m.streak.count = count;
        m.streak.last_completed_local_day = Some(LocalDay(last_day));
        m.last_warmup = Some(DailyWarmup {
            day: LocalDay(last_day),
            score: 0.5,
            extra: serde_json::Map::new(),
        });
        m
    }

    /// #257 AC1: the first-ever completion starts the streak at 1 and records
    /// the day + score. Fails if the None case seeds a wrong count or drops
    /// the warmup record.
    #[test]
    fn first_completion_starts_streak_at_one() {
        let m0 = LearnerModel::default();
        assert_eq!(m0.streak.last_completed_local_day, None);
        let m1 = apply_daily_completion(&m0, LocalDay(19_900), 0.8);
        assert_eq!(m1.streak.count, 1);
        assert_eq!(m1.streak.last_completed_local_day, Some(LocalDay(19_900)));
        let w = m1.last_warmup.as_ref().expect("warmup recorded");
        assert_eq!(w.day, LocalDay(19_900));
        assert!((w.score - 0.8).abs() < 1e-6);
        // Pure: the input model is untouched.
        assert_eq!(m0.streak.count, 0);
    }

    /// #257 AC2: completing on exactly the next local day increments the count
    /// and advances the recorded day. Fails on an off-by-one in "consecutive".
    #[test]
    fn consecutive_day_increments() {
        let m = apply_daily_completion(&streaked(4, 100), LocalDay(101), 0.6);
        assert_eq!(m.streak.count, 5);
        assert_eq!(m.streak.last_completed_local_day, Some(LocalDay(101)));
        assert_eq!(m.last_warmup.as_ref().unwrap().day, LocalDay(101));
    }

    /// #257 AC3 + AC11: a second completion on the same day never double-counts
    /// and never advances the day — it only raises the recorded daily score to
    /// the best attempt. A worse retry can't lower it.
    #[test]
    fn same_day_is_idempotent_and_keeps_best_score() {
        let m0 = streaked(4, 100);
        let m1 = apply_daily_completion(&m0, LocalDay(100), 0.9);
        assert_eq!(m1.streak.count, 4, "same day must not double-count");
        assert_eq!(m1.streak.last_completed_local_day, Some(LocalDay(100)));
        assert!((m1.last_warmup.as_ref().unwrap().score - 0.9).abs() < 1e-6);
        // A worse third attempt keeps the best of the day.
        let m2 = apply_daily_completion(&m1, LocalDay(100), 0.2);
        assert!(
            (m2.last_warmup.as_ref().unwrap().score - 0.9).abs() < 1e-6,
            "best-of-day must survive a worse retry"
        );
    }

    /// #257 AC4: any gap of a full calendar day or more resets the count to 1
    /// (today is day one of the new chain, not zero). Fails if a gap slips
    /// through as consecutive or resets to 0.
    #[test]
    fn missed_day_resets_to_one() {
        for gap in [2_i64, 3, 30, 400] {
            let m = apply_daily_completion(&streaked(9, 100), LocalDay(100 + gap), 0.7);
            assert_eq!(m.streak.count, 1, "gap of {gap} must restart at 1");
            assert_eq!(m.streak.last_completed_local_day, Some(LocalDay(100 + gap)));
        }
    }

    /// #257 AC5: a day that goes backward (clock rollback, DST fall-back,
    /// out-of-order replay) is ignored wholesale — count, recorded day, and
    /// last_warmup all keep their values. Fails if backward input decrements
    /// or rewrites anything.
    #[test]
    fn backward_day_ignored() {
        let m0 = streaked(4, 100);
        let m1 = apply_daily_completion(&m0, LocalDay(99), 1.0);
        assert_eq!(m1, m0, "a backward day must leave the model untouched");
    }

    /// #257 AC6: the transition is pure and deterministic — identical inputs
    /// produce byte-identical models, with no clock read anywhere (the
    /// signature has nowhere to put one).
    #[test]
    fn apply_daily_completion_is_pure_deterministic() {
        let m0 = streaked(2, 50);
        let a = apply_daily_completion(&m0, LocalDay(51), 0.4);
        let b = apply_daily_completion(&m0, LocalDay(51), 0.4);
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        // Every other model field is preserved unchanged — including the
        // timestamp, which this clock-free transition must not invent.
        assert_eq!(a.updated_at_epoch_secs, m0.updated_at_epoch_secs);
        assert_eq!(a.collection, m0.collection);
        assert_eq!(a.key_mastery, m0.key_mastery);
        assert_eq!(a.difficulty, m0.difficulty);
    }

    /// #257 AC10: the streak measures showing up, not performance — a 0.0
    /// score advances the chain exactly like a perfect one.
    #[test]
    fn zero_score_still_advances_streak() {
        let m = apply_daily_completion(&streaked(4, 100), LocalDay(101), 0.0);
        assert_eq!(m.streak.count, 5);
        assert_eq!(m.last_warmup.as_ref().unwrap().score, 0.0);
    }

    /// #257 AC11 + garbage hardening: out-of-range scores clamp into 0..=1 and
    /// NaN counts as 0 (same sanitization as drill accuracy) — a bad grade
    /// can't poison the stored best-of-day.
    #[test]
    fn warmup_score_is_sanitized() {
        let m1 = apply_daily_completion(&LearnerModel::default(), LocalDay(10), 7.0);
        assert_eq!(m1.last_warmup.as_ref().unwrap().score, 1.0);
        let m2 = apply_daily_completion(&m1, LocalDay(11), -3.0);
        assert_eq!(m2.last_warmup.as_ref().unwrap().score, 0.0);
        let m3 = apply_daily_completion(&m2, LocalDay(12), f32::NAN);
        assert_eq!(m3.last_warmup.as_ref().unwrap().score, 0.0);
        assert_eq!(m3.streak.count, 3, "sanitized scores still count the day");
    }

    /// Edge: the count saturates instead of overflowing u32.
    #[test]
    fn count_saturates_at_u32_max() {
        let m = apply_daily_completion(&streaked(u32::MAX, 100), LocalDay(101), 0.5);
        assert_eq!(m.streak.count, u32::MAX);
    }

    /// Edge (forward-compat): a blob saved before #257 loads with the zero
    /// streak and no warmup, and the first completion on it behaves as
    /// first-ever. Fails if the additive fields break old-blob parsing.
    #[test]
    fn model_without_streak_fields_loads_and_starts_fresh() {
        let json = r#"{ "version": 1, "collection": {}, "updated_at_epoch_secs": 4 }"#;
        let model: LearnerModel = serde_json::from_str(json).expect("pre-#257 blob parses");
        assert_eq!(model.streak, Streak::default());
        assert_eq!(model.last_warmup, None);
        let m1 = apply_daily_completion(&model, LocalDay(200), 0.5);
        assert_eq!(m1.streak.count, 1);
    }

    /// Same-day best-of-day survives a serialize→deserialize roundtrip in
    /// between (the persisted-model path the command layer will use): the
    /// prior score is read from the stored `last_warmup`, not in-memory state.
    #[test]
    fn best_of_day_reads_the_persisted_warmup() {
        let m1 = apply_daily_completion(&LearnerModel::default(), LocalDay(70), 0.9);
        let stored = serde_json::to_string(&m1).unwrap();
        let reloaded: LearnerModel = serde_json::from_str(&stored).unwrap();
        let m2 = apply_daily_completion(&reloaded, LocalDay(70), 0.3);
        assert!((m2.last_warmup.as_ref().unwrap().score - 0.9).abs() < 1e-6);
    }

    /// A `last_warmup` from a *different* day (a blob merged from sync) does
    /// not cap today's same-day score — only a same-day prior participates in
    /// best-of-day.
    #[test]
    fn best_of_day_ignores_another_days_warmup() {
        let mut m0 = streaked(3, 100);
        // Simulate a merged blob: streak says day 100, warmup is older.
        m0.last_warmup = Some(DailyWarmup {
            day: LocalDay(90),
            score: 1.0,
            extra: serde_json::Map::new(),
        });
        let m1 = apply_daily_completion(&m0, LocalDay(100), 0.4);
        let w = m1.last_warmup.as_ref().unwrap();
        assert_eq!(w.day, LocalDay(100), "the record moves to today");
        assert!(
            (w.score - 0.4).abs() < 1e-6,
            "an old day's 1.0 must not masquerade as today's best"
        );
    }
}
