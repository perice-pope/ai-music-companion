//! SQLite-backed persistence for completed practice sessions.
//!
//! Each call to [`SessionStore::save`] writes a single row: the session
//! id, the instrument played, the wall-clock start and end, and the
//! full [`SessionRecap`] serialized as JSON. Recaps are loaded back by
//! id, and a `list_recent` helper returns summaries for the history UI.
//!
//! The `bundled` feature of `rusqlite` statically links SQLite, so this
//! module works without any system-level SQLite install.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::phrase::PhraseSummary;
use crate::session::{ScoreId, SessionId, SessionRecap};

/// Raw column tuple for a `scores` row, in `SELECT` order: `id, title,
/// composer, source_filename, added_at, last_practiced_at, part_index,
/// duration_measures, music_xml`. Named to keep `ScoreStore::get` clear
/// of `clippy::type_complexity`.
type ScoreRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    i64,
    i64,
    String,
);

// ---------------------------------------------------------------------------
// Shared row decoder
// ---------------------------------------------------------------------------

/// Decode a single summary row from the `sessions` table into a
/// [`SessionSummary`]. All three list_* methods query the same eight
/// columns in the same order (`id, instrument, started_at,
/// duration_secs, phrase_count, played_secs, note_count,
/// silence_ratio`), so the decode logic lives here once.
///
/// Every malformed field escalates to [`StoreError::CorruptRow`] —
/// we never silently coerce bad data into a neutral value, because a
/// fabricated zero (phrase_count, for instance) would lie to the
/// history UI about what actually happened in a session.
fn decode_summary_row(row: &Row<'_>) -> Result<SessionSummary, StoreError> {
    // Map raw rusqlite::Error into CorruptRow so the helper's contract is
    // uniform: any row we can't decode escalates to CorruptRow, regardless
    // of whether the fault is a type mismatch (row.get), a malformed UUID,
    // a bad RFC3339 timestamp, or a signed/unsigned conversion. Letting
    // the initial reads fall through as StoreError::Sqlite would mean an
    // externally-mutated row with a NULL or wrong SQLite type reports a
    // different variant than the exact same row with a parseable-but-
    // impossible value — callers shouldn't need to distinguish those.
    let id_str: String = row
        .get(0)
        .map_err(|e| StoreError::CorruptRow(format!("invalid id column: {e}")))?;
    let instrument: String = row.get(1).map_err(|e| {
        StoreError::CorruptRow(format!(
            "invalid instrument column for session {id_str}: {e}"
        ))
    })?;
    let started_at_str: String = row.get(2).map_err(|e| {
        StoreError::CorruptRow(format!(
            "invalid started_at column for session {id_str}: {e}"
        ))
    })?;
    let duration_secs: f64 = row.get(3).map_err(|e| {
        StoreError::CorruptRow(format!(
            "invalid duration_secs column for session {id_str}: {e}"
        ))
    })?;
    let phrase_count: i64 = row.get(4).map_err(|e| {
        StoreError::CorruptRow(format!(
            "invalid phrase_count column for session {id_str}: {e}"
        ))
    })?;
    // #449 T1 integrity aggregates — NULL (None) on rows persisted before the
    // v2 migration or before the close-time computation ran: honest absence,
    // never a fabricated zero (a fake 0.0 silence_ratio would flatter a
    // walk-away session).
    let played_secs: Option<f64> = row.get(5).map_err(|e| {
        StoreError::CorruptRow(format!(
            "invalid played_secs column for session {id_str}: {e}"
        ))
    })?;
    let note_count_raw: Option<i64> = row.get(6).map_err(|e| {
        StoreError::CorruptRow(format!(
            "invalid note_count column for session {id_str}: {e}"
        ))
    })?;
    let silence_ratio: Option<f64> = row.get(7).map_err(|e| {
        StoreError::CorruptRow(format!(
            "invalid silence_ratio column for session {id_str}: {e}"
        ))
    })?;
    let note_count = note_count_raw
        .map(|n| {
            u64::try_from(n).map_err(|_| {
                StoreError::CorruptRow(format!("negative note_count {n} for session {id_str}"))
            })
        })
        .transpose()?;

    let id: SessionId = id_str.parse().map_err(|e: uuid::Error| {
        StoreError::CorruptRow(format!("invalid session id {id_str}: {e}"))
    })?;
    let started_at = DateTime::parse_from_rfc3339(&started_at_str)
        .map_err(|e| {
            StoreError::CorruptRow(format!("invalid RFC3339 started_at {started_at_str}: {e}"))
        })?
        .with_timezone(&Utc);
    let phrase_count = usize::try_from(phrase_count).map_err(|_| {
        StoreError::CorruptRow(format!(
            "invalid phrase_count {phrase_count} for session {id_str}"
        ))
    })?;
    Ok(SessionSummary {
        id,
        instrument,
        started_at,
        duration_secs,
        phrase_count,
        played_secs,
        note_count,
        silence_ratio,
    })
}

/// Escalate a signed SQLite `COUNT(*)` result into an unsigned count.
///
/// A negative value from SQLite is impossible under normal operation,
/// so if one appears it's data corruption — we surface it as
/// [`StoreError::CorruptRow`] rather than silently returning zero,
/// which would lie to callers about the size of their history.
///
/// Extracted so the regression test can exercise the conversion logic
/// directly. Calling `count_sessions()` inserts rows through the real
/// table, which can't actually produce a negative count; testing the
/// helper in isolation is the only way to keep this code path covered.
fn decode_session_count(count: i64) -> Result<usize, StoreError> {
    usize::try_from(count).map_err(|_| {
        StoreError::CorruptRow(format!("negative session count {count} from COUNT(*)"))
    })
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors emitted by [`SessionStore`].
#[derive(Debug, Error)]
pub enum StoreError {
    /// Raw SQLite error (migration, prepare, execute, etc.).
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Failure serializing or deserializing the JSON recap blob.
    #[error("serde_json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Requested session id not present in the database.
    #[error("session not found: {0}")]
    NotFound(String),
    /// Platform could not provide a data directory for the default
    /// path (extremely rare — headless container, missing HOME, ...).
    #[error("data directory not available")]
    NoDataDir,
    /// IO error creating the parent directory for the default path.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A stored row could not be decoded (corrupt id, bad timestamp).
    /// Should not occur for data this crate wrote; guards against
    /// externally-mutated databases.
    #[error("corrupt row in sessions table: {0}")]
    CorruptRow(String),
}

// ---------------------------------------------------------------------------
// Summary row returned by list_recent
// ---------------------------------------------------------------------------

/// A loaded session with authoritative persisted timestamps.
///
/// `load()` returns this rather than a bare `SessionRecap` so callers can
/// see exactly when the session happened — the timestamps `save()` wrote
/// would otherwise be effectively write-only, forcing callers to
/// reconstruct them by adding `duration_secs` to a cached `started_at`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredSession {
    /// Unique session id.
    pub id: SessionId,
    /// Wall-clock start of the session.
    pub started_at: DateTime<Utc>,
    /// Wall-clock end of the session.
    pub ended_at: DateTime<Utc>,
    /// The persisted recap.
    pub recap: SessionRecap,
}

/// Lightweight summary row for the session history UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    /// Unique session id.
    pub id: SessionId,
    /// Instrument played.
    pub instrument: String,
    /// When the session started.
    pub started_at: DateTime<Utc>,
    /// Total session duration in seconds.
    pub duration_secs: f64,
    /// Number of phrases played in the session.
    pub phrase_count: usize,
    /// #449 T1 (§1b): Σ phrase durations — the #445-6b/#451 played clock,
    /// persisted at session close. `None` on rows that predate the column
    /// or the close-time computation (honest absence, never zero).
    pub played_secs: Option<f64>,
    /// #449 T1 (§1b): Σ voiced events detected across phrases. `None` as above.
    pub note_count: Option<u64>,
    /// #449 T1 (§1b): `1 − played/wall`, clamped to `[0, 1]`; `1.0` when the
    /// wall clock was zero. `None` as above.
    pub silence_ratio: Option<f64>,
}

/// Coarse, self-reported experience level. Deliberately three buckets, not a
/// fine scale — the personalization spine uses it to shape coaching vocabulary
/// and depth, never to grade the student.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceLevel {
    /// New to the instrument (or returning after a long break).
    #[default]
    Beginner,
    /// Comfortable with fundamentals, building repertoire.
    Intermediate,
    /// Fluent; refining musicianship and advanced technique.
    Advanced,
}

impl ExperienceLevel {
    /// snake_case wire string, matching the serde representation and the
    /// `experience` CHECK constraint on the Supabase `taste_profile` table.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Beginner => "beginner",
            Self::Intermediate => "intermediate",
            Self::Advanced => "advanced",
        }
    }
}

/// The student's stated taste profile — who they are, not what they played.
///
/// This is the personalization spine's one student record (see
/// `docs/architecture/platform-spine-personalization.md`): genres, favourite
/// artists, goals, and coarse experience level, captured at onboarding and
/// editable any time. It is **distinct** from the measured
/// [`crate::fingerprint::MusicalFingerprint`] — preferences the coach may use to
/// *frame*, never performance facts it may *assert*.
///
/// Stored locally as one JSON-backed row per user. `#[serde(default)]` on every
/// field keeps the JSON forward-compatible: a profile serialised before a field
/// existed still loads (the missing field defaults), so adding preference
/// dimensions later needs no DB migration — exactly the contract the spine
/// relies on for "the record is not rebuilt across phases".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TasteProfile {
    /// Stated genres, e.g. `["hip-hop", "film score", "gospel"]`.
    #[serde(default)]
    pub genres: Vec<String>,
    /// Stated favourite artists, e.g. `["Kendrick Lamar", "Hans Zimmer"]`.
    #[serde(default)]
    pub artists: Vec<String>,
    /// Why the student is here, e.g. `["audition prep", "play in church band"]`.
    #[serde(default)]
    pub goals: Vec<String>,
    /// Coarse self-reported experience level.
    #[serde(default)]
    pub experience: ExperienceLevel,
    /// Reuses the shipped under-13 precedent (teacher-audit.md) instead of a
    /// birthdate: for minors we keep the profile minimal and parent-visible.
    #[serde(default)]
    pub is_under_13: bool,
}

/// A score in the library (MusicXML file).
///
/// Stores the loaded score metadata and parsed MusicXML content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreLibraryEntry {
    /// Unique score id.
    pub id: ScoreId,
    /// Title of the piece.
    pub title: String,
    /// Composer, if known.
    pub composer: Option<String>,
    /// Original filename for display.
    pub source_filename: String,
    /// When the score was added to the library.
    pub added_at: DateTime<Utc>,
    /// Last time this score was used in a session.
    pub last_practiced_at: Option<DateTime<Utc>>,
    /// Part index (0 for single-part scores).
    pub part_index: usize,
    /// Number of measures in the score.
    pub duration_measures: usize,
    /// Raw MusicXML content.
    pub music_xml: String,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    instrument TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    duration_secs REAL NOT NULL,
    phrase_count INTEGER NOT NULL,
    recap_json TEXT NOT NULL,
    score_id TEXT REFERENCES scores(id) ON DELETE SET NULL,
    played_secs REAL,
    note_count INTEGER,
    silence_ratio REAL
);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at DESC);
CREATE TABLE IF NOT EXISTS scores (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    composer TEXT,
    source_filename TEXT NOT NULL,
    added_at TEXT NOT NULL,
    last_practiced_at TEXT,
    part_index INTEGER NOT NULL DEFAULT 0,
    duration_measures INTEGER NOT NULL DEFAULT 0,
    music_xml TEXT NOT NULL,
    content_hash TEXT
);
CREATE INDEX IF NOT EXISTS idx_scores_last_practiced ON scores(last_practiced_at DESC, added_at DESC);
CREATE TABLE IF NOT EXISTS taste_profile (
    user_id TEXT PRIMARY KEY,
    profile_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS learner_model (
    user_id TEXT PRIMARY KEY,
    model_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS session_phrases (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    phrase_index INTEGER NOT NULL,
    note_count INTEGER NOT NULL,
    start_secs REAL NOT NULL,
    end_secs REAL NOT NULL,
    phrase_json TEXT NOT NULL,
    PRIMARY KEY (session_id, phrase_index)
);
CREATE INDEX IF NOT EXISTS idx_session_phrases_session ON session_phrases(session_id);
-- The exercise log (#252 self-improvement): every exercise the engine
-- GENERATES, with what came back. Append-only evidence of which material
-- works — the raw feed for tuning the coach. Local-first like everything.
CREATE TABLE IF NOT EXISTS exercise_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    logged_at TEXT NOT NULL,
    source TEXT NOT NULL,
    label TEXT NOT NULL,
    spec_json TEXT NOT NULL,
    seed INTEGER NOT NULL,
    difficulty INTEGER NOT NULL,
    tonic INTEGER NOT NULL,
    accuracy REAL,
    spec_hash TEXT
);
CREATE INDEX IF NOT EXISTS idx_exercise_log_source ON exercise_log(source);
-- HOW the student practiced (#449 T1): tool usage during a session.
-- Append-only, event-sourced, one clock (seconds from session start — the
-- same clock #451's played-time copy uses). Local-first like everything;
-- leaves the device only under the T2 enrollment sync opt-in, disclosed
-- in ConnectionsPrivacy BEFORE it can sync. Until T2 lands, this table
-- syncs NOTHING.
CREATE TABLE IF NOT EXISTS practice_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    at_secs REAL NOT NULL,
    kind TEXT NOT NULL,
    params_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_practice_events_session
    ON practice_events(session_id, at_secs);
-- #419 S4: named opener recipes the player chose to keep. items_json is
-- the serialized Vec<StarterItem>; parse failures on read are SKIPPED
-- (a stale row must never break the panel), never surfaced.
CREATE TABLE IF NOT EXISTS starter_recipes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    name TEXT NOT NULL,
    items_json TEXT NOT NULL,
    direction TEXT NOT NULL
);
";

/// One row of the exercise log (#252 self-improvement): what the engine
/// generated, and what came back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExerciseLogEntry {
    /// Where it came from: "lesson", "explore", "explore_chip", "lift".
    pub source: String,
    /// The human label F1 generated (names the material + knobs).
    pub label: String,
    /// The full VariationSpec as JSON — replayable and analyzable.
    pub spec_json: String,
    pub seed: u64,
    pub difficulty: u8,
    pub tonic: u8,
    /// Graded accuracy 0..1; `None` = generated but never graded.
    pub accuracy: Option<f64>,
}

/// #453 S1: one exercise-log row WITH its write stamp — the history
/// analyzer's input. `logged_at` is the stored RFC3339 TEXT, surfaced
/// unparsed on purpose: the store wrote it, but a copied/edited/corrupt
/// database may hold anything, so consumers parse defensively and skip
/// garbage rather than trust the column.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedExerciseLogEntry {
    /// RFC3339 write stamp as stored — treat as unparsed input.
    pub logged_at: String,
    pub entry: ExerciseLogEntry,
}

/// #419 S4: one saved opener recipe row, as stored. `items_json` is
/// opaque here — the starter vocabulary lives above the store.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedRecipeRow {
    pub id: i64,
    pub name: String,
    pub items_json: String,
    pub direction: String,
}

/// The `user_id` used for the single local taste profile before any cloud
/// account exists. Local-first: the profile is captured at onboarding with no
/// sign-in required; if the user later opts into sync, the row is projected up
/// keyed on their real `profiles.id`. Mirrors the local-then-synced model the
/// session data already uses.
pub const LOCAL_TASTE_PROFILE_USER_ID: &str = "local";

/// Add `column` to `table` only if absent. SQLite's
/// `ALTER TABLE ADD COLUMN` is not idempotent, so we check `table_info`
/// first — keeping `migrate()` safe to run on fresh and already-migrated
/// databases alike. `table`/`column`/`decl` are internal constants, never
/// user input.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> Result<(), StoreError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    drop(stmt);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))?;
    }
    Ok(())
}

/// 64-bit FNV-1a over the stored MusicXML — the dedup key for score imports
/// (#385). Not cryptographic on purpose: the only consequence of a collision
/// is an import reusing another entry, so the bar is "astronomically unlikely
/// across a personal library", which 64 bits clears without adding a hash
/// dependency. Stability across releases matters more here than strength —
/// hashes persist in the DB, so `std`'s `DefaultHasher` (unstable across Rust
/// versions by contract) would silently orphan old rows.
fn score_content_hash(music_xml: &str) -> String {
    fnv1a_64_hex(music_xml)
}

/// 64-bit FNV-1a over a string's bytes, as 16 hex chars — the one hashing
/// primitive both persisted hash columns share (`scores.content_hash`,
/// `exercise_log.spec_hash`). See [`score_content_hash`] for why FNV over a
/// crypto hash or `DefaultHasher`.
fn fnv1a_64_hex(input: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// #449 T1 (§1c): the exercise retry key — FNV-1a 64 of the row's `spec_json`
/// bytes as-is. The datamodel doc says "tonic excluded": `spec_json` (a
/// serialized `VariationSpec`) carries no tonic — tonic is its own column —
/// so hashing the bytes verbatim already gives the RV grouping (same cell,
/// different key ⇒ same `spec_hash`, different `tonic`). Stable across
/// releases by the same argument as [`score_content_hash`]: these hashes
/// persist in the DB.
pub fn exercise_spec_hash(spec_json: &str) -> String {
    fnv1a_64_hex(spec_json)
}

// ---------------------------------------------------------------------------
// SessionStore
// ---------------------------------------------------------------------------

/// Session-level debug metadata stored on the `sessions` row, surfaced for
/// diagnosing user-reported issues (which build, free-play vs score, etc.).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMeta {
    /// App version that produced the session (`None` on older rows).
    pub app_version: Option<String>,
    /// Practice mode label (e.g. free-play vs a score-following mode).
    pub practice_mode: Option<String>,
    /// Id of the score practised, if any.
    pub score_id: Option<String>,
}

/// #449 T1 (§1b): the anti-fudge aggregates for one session, computed once,
/// in Rust, at session close — never re-derived downstream (three dashboards
/// re-deriving them would drift apart).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionIntegrity {
    /// Σ phrase `(end_time − start_time)` — the #445-6b/#451 played clock.
    pub played_secs: f64,
    /// Σ phrase `note_count`: voiced events actually detected.
    pub note_count: u64,
    /// `1 − played_secs / wall_secs`, clamped to `[0, 1]`.
    pub silence_ratio: f64,
}

/// Compute the session-integrity aggregates from a completed session's
/// phrases and its wall-clock duration.
///
/// Rules (documented once, here, so they can never fork):
/// - `played_secs` sums each phrase's `(end_time − start_time)`, clamping a
///   (corrupt) negative span to zero rather than letting it eat real time.
/// - `note_count` sums the per-phrase voiced counts.
/// - `silence_ratio` is `1 − played/wall` clamped to `[0, 1]`; a session with
///   `wall_secs <= 0` reports `1.0` — a zero-length wall has, vacuously, no
///   played sound, and `1.0` keeps the F1 walk-away flag monotone instead of
///   producing NaN/negative garbage.
pub fn session_integrity(phrases: &[PhraseSummary], wall_secs: f64) -> SessionIntegrity {
    let played_secs: f64 = phrases
        .iter()
        .map(|p| (p.end_time - p.start_time).max(0.0))
        .sum();
    let note_count: u64 = phrases.iter().map(|p| p.note_count as u64).sum();
    let silence_ratio = if wall_secs <= 0.0 {
        1.0
    } else {
        (1.0 - played_secs / wall_secs).clamp(0.0, 1.0)
    };
    SessionIntegrity {
        played_secs,
        note_count,
        silence_ratio,
    }
}

/// One row of the `practice_events` tool-usage journal (#449 T1 §1a), as
/// stored. `params_json` is opaque here — the event vocabulary lives in the
/// command layer that writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct PracticeEventRow {
    pub id: i64,
    pub session_id: String,
    /// Seconds from session start (one clock — offsets from
    /// `sessions.started_at`, range-joinable against `session_phrases`).
    pub at_secs: f64,
    pub kind: String,
    pub params_json: String,
}

/// SQLite-backed store of completed practice sessions.
pub struct SessionStore {
    conn: Connection,
}

impl SessionStore {
    /// Open (or create) the database file at `path`.
    ///
    /// Runs migrations idempotently — safe to call repeatedly across
    /// app launches.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Open an ephemeral in-memory database. Used by tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Execute the schema migrations. Idempotent.
    fn migrate(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(SCHEMA)?;
        // Versioned migrations for changes `CREATE TABLE IF NOT EXISTS` can't
        // express — e.g. adding a column to a table that already exists.
        // `PRAGMA user_version` records progress so each step runs once per DB.
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            // v1: session-level debug columns. Databases created before these
            // (and before `score_id` was added to SCHEMA) gain them now; a
            // fresh DB already has the column, so the guard skips it.
            add_column_if_missing(&self.conn, "sessions", "score_id", "TEXT")?;
            add_column_if_missing(&self.conn, "sessions", "app_version", "TEXT")?;
            add_column_if_missing(&self.conn, "sessions", "practice_mode", "TEXT")?;
            self.conn.execute_batch("PRAGMA user_version = 1;")?;
        }
        if version < 2 {
            // v2 (#449 T1): the session-integrity aggregates (§1b) and the
            // exercise retry key (§1c). Fresh DBs already have all four via
            // SCHEMA; older DBs gain them here. NULL on pre-existing session
            // rows is honest absence (the close-time computation never ran),
            // never a fabricated zero.
            add_column_if_missing(&self.conn, "sessions", "played_secs", "REAL")?;
            add_column_if_missing(&self.conn, "sessions", "note_count", "INTEGER")?;
            add_column_if_missing(&self.conn, "sessions", "silence_ratio", "REAL")?;
            add_column_if_missing(&self.conn, "exercise_log", "spec_hash", "TEXT")?;
            // Backfill spec_hash in one pass so old rows group with new ones.
            // Cost: one FNV over each spec_json already in memory — a few ms
            // even for thousands of rows — and the user_version guard means
            // this runs exactly once per database.
            self.backfill_spec_hashes()?;
            self.conn.execute_batch("PRAGMA user_version = 2;")?;
        }
        Ok(())
    }

    /// Hash any `exercise_log` rows the v2 migration left without a
    /// `spec_hash`. Only ever touches NULL rows, so repeated calls are
    /// no-ops (mirrors `ScoreStore::backfill_content_hashes`).
    fn backfill_spec_hashes(&self) -> Result<(), StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, spec_json FROM exercise_log WHERE spec_hash IS NULL")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        for (id, spec_json) in rows {
            self.conn.execute(
                "UPDATE exercise_log SET spec_hash = ?1 WHERE id = ?2",
                params![exercise_spec_hash(&spec_json), id],
            )?;
        }
        Ok(())
    }

    /// Persist a session.
    ///
    /// `started_at` and `ended_at` are serialized as RFC3339 strings.
    /// The full [`SessionRecap`] is stored as JSON so future schema
    /// changes to the recap can be absorbed without a DB migration.
    pub fn save(
        &self,
        id: SessionId,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
        recap: &SessionRecap,
    ) -> Result<(), StoreError> {
        let recap_json = serde_json::to_string(recap)?;
        let phrase_count = i64::try_from(recap.phrase_count).unwrap_or(i64::MAX);

        // Derive the authoritative `duration_secs` from the timestamps
        // we're about to persist, NOT from `recap.duration_secs`. The
        // recap blob is opaque LLM-adjacent data; internal row fields
        // must stay consistent with each other so `list_recent` doesn't
        // report a different number than what `ended_at - started_at`
        // implies.
        let duration_secs = (ended_at - started_at).num_milliseconds() as f64 / 1000.0;

        self.conn.execute(
            "INSERT OR REPLACE INTO sessions \
             (id, instrument, started_at, ended_at, duration_secs, phrase_count, recap_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id.as_str(),
                recap.instrument,
                started_at.to_rfc3339(),
                ended_at.to_rfc3339(),
                duration_secs,
                phrase_count,
                recap_json,
            ],
        )?;
        Ok(())
    }

    /// Attach session-level debug metadata to an already-saved session row.
    ///
    /// Kept separate from [`save`](Self::save) so its many callers stay
    /// unchanged. Best-effort: any `None` leaves that column NULL, and an
    /// unknown id updates zero rows. Call right after `save` so the row exists.
    pub fn record_session_meta(
        &self,
        id: SessionId,
        app_version: Option<&str>,
        practice_mode: Option<&str>,
        score_id: Option<&str>,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE sessions SET app_version = ?2, practice_mode = ?3, score_id = ?4 \
             WHERE id = ?1",
            params![id.as_str(), app_version, practice_mode, score_id],
        )?;
        Ok(())
    }

    /// Read the session-level debug metadata `(app_version, practice_mode,
    /// score_id)` for a session. Each element is `None` when unset.
    pub fn session_meta(&self, id: SessionId) -> Result<SessionMeta, StoreError> {
        let meta = self.conn.query_row(
            "SELECT app_version, practice_mode, score_id FROM sessions WHERE id = ?1",
            params![id.as_str()],
            |r| {
                Ok(SessionMeta {
                    app_version: r.get(0)?,
                    practice_mode: r.get(1)?,
                    score_id: r.get(2)?,
                })
            },
        )?;
        Ok(meta)
    }

    /// #449 T1 (§1b): persist the close-time integrity aggregates onto an
    /// already-saved session row. Kept separate from [`save`](Self::save) —
    /// same shape as [`record_session_meta`](Self::record_session_meta):
    /// best-effort caller, unknown id updates zero rows.
    pub fn record_session_integrity(
        &self,
        id: SessionId,
        integrity: &SessionIntegrity,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE sessions SET played_secs = ?2, note_count = ?3, silence_ratio = ?4 \
             WHERE id = ?1",
            params![
                id.as_str(),
                integrity.played_secs,
                i64::try_from(integrity.note_count).unwrap_or(i64::MAX),
                integrity.silence_ratio,
            ],
        )?;
        Ok(())
    }

    /// #449 T1 (§1a): append one tool-usage event to the journal.
    ///
    /// Append-only — corrections are new events, never UPDATEs (the
    /// `exercise_log` discipline). The caller supplies `at_secs` already on
    /// the one session clock (seconds from `sessions.started_at`). This is
    /// the raw store write; the never-fails wrapper lives in the command
    /// layer (`log_practice_event_best_effort`), mirroring how
    /// `log_exercise` / `log_exercise_best_effort` split.
    pub fn log_practice_event(
        &self,
        session_id: &str,
        at_secs: f64,
        kind: &str,
        params_json: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO practice_events (session_id, at_secs, kind, params_json) \
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, at_secs, kind, params_json],
        )?;
        Ok(())
    }

    /// #449 T1: the journal for one session, in session-clock order (ties
    /// break on insert order). Read off the hot path only — history surfaces
    /// and, later, the T2 sync projection.
    pub fn list_practice_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<PracticeEventRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, at_secs, kind, params_json \
             FROM practice_events WHERE session_id = ?1 ORDER BY at_secs, id",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(PracticeEventRow {
                id: row.get(0)?,
                session_id: row.get(1)?,
                at_secs: row.get(2)?,
                kind: row.get(3)?,
                params_json: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// #449 T1: total journal size across all sessions. Same corrupt-count
    /// escalation as [`count_sessions`](Self::count_sessions).
    pub fn count_practice_events(&self) -> Result<usize, StoreError> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM practice_events", [], |row| row.get(0))?;
        decode_session_count(count)
    }

    /// Failure injection for tests ONLY (#449 T1 review MF3): drop the
    /// `practice_events` table so the next journal write fails, letting the
    /// desktop crate prove its best-effort writer swallows a real store
    /// error instead of panicking or surfacing it into the practice loop.
    /// Compiled only under the `test-support` feature, which only downstream
    /// dev-dependencies enable — a shipping build cannot contain it.
    #[cfg(feature = "test-support")]
    pub fn break_practice_events_for_tests(&self) {
        self.conn
            .execute_batch("DROP TABLE IF EXISTS practice_events;")
            .expect("dropping the journal table in a test fixture");
    }

    /// Persist the per-phrase metrics for a session.
    ///
    /// Stored alongside the session row (FK with `ON DELETE CASCADE`) so a
    /// user-reported issue can be debugged from the raw phrase data —
    /// pitch/cents/confidence/timing — not just the summary recap. Each phrase
    /// is kept as JSON (additive `serde` keeps old rows readable) plus a few
    /// indexed columns for cheap querying. Idempotent per
    /// `(session_id, phrase_index)` via `INSERT OR REPLACE`.
    pub fn save_phrases(
        &self,
        session_id: SessionId,
        phrases: &[PhraseSummary],
    ) -> Result<(), StoreError> {
        let id_str = session_id.as_str();
        for phrase in phrases {
            let phrase_json = serde_json::to_string(phrase)?;
            self.conn.execute(
                "INSERT OR REPLACE INTO session_phrases \
                 (session_id, phrase_index, note_count, start_secs, end_secs, phrase_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id_str,
                    i64::try_from(phrase.phrase_index).unwrap_or(i64::MAX),
                    i64::try_from(phrase.note_count).unwrap_or(i64::MAX),
                    phrase.start_time,
                    phrase.end_time,
                    phrase_json,
                ],
            )?;
        }
        Ok(())
    }

    /// Load the persisted phrases for a session, ordered by phrase index.
    ///
    /// Returns an empty vec for a session with no stored phrases (e.g. a row
    /// written before phrase persistence shipped).
    pub fn load_phrases(&self, session_id: SessionId) -> Result<Vec<PhraseSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT phrase_json FROM session_phrases \
             WHERE session_id = ?1 ORDER BY phrase_index",
        )?;
        let rows = stmt.query_map(params![session_id.as_str()], |r| r.get::<_, String>(0))?;
        let mut phrases = Vec::new();
        for row in rows {
            phrases.push(serde_json::from_str(&row?)?);
        }
        Ok(phrases)
    }

    /// Load the recap for a specific session id.
    ///
    /// Returns [`StoreError::NotFound`] if the id is unknown.
    /// Load a full stored session (recap + persisted timestamps) by id.
    pub fn load(&self, id: SessionId) -> Result<StoredSession, StoreError> {
        let id_str = id.as_str();
        let row: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT recap_json, started_at, ended_at FROM sessions WHERE id = ?1",
                params![id_str],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;

        match row {
            Some((json, started_at_str, ended_at_str)) => {
                let recap: SessionRecap = serde_json::from_str(&json)?;
                let started_at = DateTime::parse_from_rfc3339(&started_at_str)
                    .map_err(|e| {
                        StoreError::CorruptRow(format!(
                            "invalid RFC3339 started_at {started_at_str}: {e}"
                        ))
                    })?
                    .with_timezone(&Utc);
                let ended_at = DateTime::parse_from_rfc3339(&ended_at_str)
                    .map_err(|e| {
                        StoreError::CorruptRow(format!(
                            "invalid RFC3339 ended_at {ended_at_str}: {e}"
                        ))
                    })?
                    .with_timezone(&Utc);
                Ok(StoredSession {
                    id,
                    started_at,
                    ended_at,
                    recap,
                })
            }
            None => Err(StoreError::NotFound(id_str)),
        }
    }

    /// Convenience: load just the recap when the timestamps aren't
    /// needed. Thin wrapper over [`Self::load`] so callers that only
    /// care about the recap body don't have to pull it out themselves.
    pub fn load_recap(&self, id: SessionId) -> Result<SessionRecap, StoreError> {
        self.load(id).map(|s| s.recap)
    }

    /// Return up to `limit` session summaries, most recent first
    /// (ordered by `started_at DESC`).
    pub fn list_recent(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, instrument, started_at, duration_secs, phrase_count, \
                    played_secs, note_count, silence_ratio \
             FROM sessions ORDER BY started_at DESC LIMIT ?1",
        )?;
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut rows = stmt.query(params![limit_i64])?;

        let mut summaries = Vec::new();
        while let Some(row) = rows.next()? {
            summaries.push(decode_summary_row(row)?);
        }
        Ok(summaries)
    }

    /// Return session summaries filtered by instrument, ordered by `started_at DESC`.
    ///
    /// If `instrument` is `None`, no instrument filter is applied (returns all).
    /// If `instrument` is `Some("")`, only sessions with empty instrument are returned.
    pub fn list_by_instrument(
        &self,
        instrument: Option<&str>,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let query = if instrument.is_some() {
            "SELECT id, instrument, started_at, duration_secs, phrase_count, \
                    played_secs, note_count, silence_ratio \
             FROM sessions WHERE instrument = ?1 ORDER BY started_at DESC"
        } else {
            "SELECT id, instrument, started_at, duration_secs, phrase_count, \
                    played_secs, note_count, silence_ratio \
             FROM sessions ORDER BY started_at DESC"
        };

        let mut stmt = self.conn.prepare(query)?;
        let mut rows = if let Some(inst) = instrument {
            stmt.query(params![inst])?
        } else {
            stmt.query([])?
        };

        let mut summaries = Vec::new();
        while let Some(row) = rows.next()? {
            summaries.push(decode_summary_row(row)?);
        }
        Ok(summaries)
    }

    /// Return session summaries within a date range [start_at, end_at), ordered by `started_at DESC`.
    ///
    /// If `start_at` is `None`, no lower bound is applied.
    /// If `end_at` is `None`, no upper bound is applied.
    pub fn list_by_date_range(
        &self,
        start_at: Option<DateTime<Utc>>,
        end_at: Option<DateTime<Utc>>,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let (query, start_str, end_str) = match (&start_at, &end_at) {
            (Some(s), Some(e)) => (
                "SELECT id, instrument, started_at, duration_secs, phrase_count, \
                    played_secs, note_count, silence_ratio \
                 FROM sessions WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at DESC",
                Some(s.to_rfc3339()),
                Some(e.to_rfc3339()),
            ),
            (Some(s), None) => (
                "SELECT id, instrument, started_at, duration_secs, phrase_count, \
                    played_secs, note_count, silence_ratio \
                 FROM sessions WHERE started_at >= ?1 ORDER BY started_at DESC",
                Some(s.to_rfc3339()),
                None,
            ),
            (None, Some(e)) => (
                "SELECT id, instrument, started_at, duration_secs, phrase_count, \
                    played_secs, note_count, silence_ratio \
                 FROM sessions WHERE started_at < ?1 ORDER BY started_at DESC",
                Some(e.to_rfc3339()),
                None,
            ),
            (None, None) => (
                "SELECT id, instrument, started_at, duration_secs, phrase_count, \
                    played_secs, note_count, silence_ratio \
                 FROM sessions ORDER BY started_at DESC",
                None,
                None,
            ),
        };

        let mut stmt = self.conn.prepare(query)?;
        let mut rows = match (&start_str, &end_str) {
            (Some(s), Some(e)) => stmt.query(params![s.as_str(), e.as_str()])?,
            (Some(s), None) => stmt.query(params![s.as_str()])?,
            (None, Some(e)) => stmt.query(params![e.as_str()])?,
            (None, None) => stmt.query([])?,
        };

        let mut summaries = Vec::new();
        while let Some(row) = rows.next()? {
            summaries.push(decode_summary_row(row)?);
        }
        Ok(summaries)
    }

    /// Count total sessions in the database.
    ///
    /// A negative count from SQLite is impossible under normal
    /// operation, but if one appears it's data corruption — we
    /// escalate to [`StoreError::CorruptRow`] instead of silently
    /// returning zero, which would lie to callers about the size
    /// of their history. Matches the pattern used by every other
    /// decode path in this module.
    pub fn count_sessions(&self) -> Result<usize, StoreError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        decode_session_count(count)
    }

    /// Sum total practice duration in seconds across all sessions.
    pub fn total_duration_secs(&self) -> Result<f64, StoreError> {
        let total: Option<f64> =
            self.conn
                .query_row("SELECT SUM(duration_secs) FROM sessions", [], |row| {
                    row.get(0)
                })?;
        Ok(total.unwrap_or(0.0))
    }

    /// Fetch the locally-stored taste profile for `user_id`, if one exists.
    ///
    /// Returns `Ok(None)` when no profile has been captured yet (cold start) —
    /// that is a normal, non-error state the onboarding flow keys off of. The
    /// profile body is decoded from JSON; `#[serde(default)]` on
    /// [`TasteProfile`] means a row written before a field existed still loads.
    pub fn get_taste_profile(&self, user_id: &str) -> Result<Option<TasteProfile>, StoreError> {
        let row: Option<String> = self
            .conn
            .query_row(
                "SELECT profile_json FROM taste_profile WHERE user_id = ?1",
                params![user_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        match row {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    /// Insert or replace the taste profile for `user_id`.
    ///
    /// Upsert (not append): one row per user, so onboarding and every later
    /// edit go through the same path. The full profile is stored as JSON so new
    /// preference dimensions are absorbed without a DB migration — the same
    /// forward-compat strategy the session recap uses.
    pub fn upsert_taste_profile(
        &self,
        user_id: &str,
        profile: &TasteProfile,
    ) -> Result<(), StoreError> {
        let profile_json = serde_json::to_string(profile)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO taste_profile (user_id, profile_json, updated_at) \
             VALUES (?1, ?2, ?3)",
            params![user_id, profile_json, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Load the learner model for `user_id` (#252 F2).
    ///
    /// `Ok(None)` on cold start — every feature treats that as
    /// [`LearnerModel::default`]. Same single-row-JSON strategy as the taste
    /// profile: the blob is versioned and forward-compatible, so schema growth
    /// needs no DB migration.
    pub fn get_learner_model(
        &self,
        user_id: &str,
    ) -> Result<Option<crate::learner::LearnerModel>, StoreError> {
        let row: Option<String> = self
            .conn
            .query_row(
                "SELECT model_json FROM learner_model WHERE user_id = ?1",
                params![user_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        match row {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    /// Insert or replace the learner model for `user_id` (#252 F2). One row per
    /// user; every pure transition writes back through this same path.
    pub fn upsert_learner_model(
        &self,
        user_id: &str,
        model: &crate::learner::LearnerModel,
    ) -> Result<(), StoreError> {
        let model_json = serde_json::to_string(model)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO learner_model (user_id, model_json, updated_at) \
             VALUES (?1, ?2, ?3)",
            params![user_id, model_json, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Append one generated exercise + its outcome to the exercise log
    /// (#252 self-improvement). `accuracy` is `None` for exercises that were
    /// generated but never graded (explored, abandoned) — absence is itself a
    /// signal. Append-only; never blocks the practice loop on failure.
    pub fn log_exercise(&self, entry: &ExerciseLogEntry) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO exercise_log              (logged_at, source, label, spec_json, seed, difficulty, tonic, accuracy, spec_hash)              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                Utc::now().to_rfc3339(),
                entry.source,
                entry.label,
                entry.spec_json,
                entry.seed as i64,
                i64::from(entry.difficulty),
                i64::from(entry.tonic),
                entry.accuracy,
                // #449 T1: the retry key, written at insert so every new row
                // groups without a spec_json parse (see exercise_spec_hash).
                exercise_spec_hash(&entry.spec_json),
            ],
        )?;
        Ok(())
    }

    /// Read the whole exercise log, oldest → newest (the analyzer's input).
    pub fn list_exercise_log(&self) -> Result<Vec<ExerciseLogEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT source, label, spec_json, seed, difficulty, tonic, accuracy              FROM exercise_log ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ExerciseLogEntry {
                source: row.get(0)?,
                label: row.get(1)?,
                spec_json: row.get(2)?,
                seed: row.get::<_, i64>(3)? as u64,
                difficulty: row.get::<_, i64>(4)?.clamp(0, 255) as u8,
                tonic: row.get::<_, i64>(5)?.clamp(0, 255) as u8,
                accuracy: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// #453 S1: the whole exercise log with write stamps, oldest → newest —
    /// the history analyzer's input. Same rows as [`Self::list_exercise_log`]
    /// plus `logged_at`.
    pub fn list_exercise_log_timed(&self) -> Result<Vec<TimedExerciseLogEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT logged_at, source, label, spec_json, seed, difficulty, tonic, accuracy \
             FROM exercise_log ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TimedExerciseLogEntry {
                logged_at: row.get(0)?,
                entry: ExerciseLogEntry {
                    source: row.get(1)?,
                    label: row.get(2)?,
                    spec_json: row.get(3)?,
                    seed: row.get::<_, i64>(4)? as u64,
                    difficulty: row.get::<_, i64>(5)?.clamp(0, 255) as u8,
                    tonic: row.get::<_, i64>(6)?.clamp(0, 255) as u8,
                    accuracy: row.get(7)?,
                },
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// #419 S4: keep a named opener recipe. Returns the new row id.
    pub fn save_recipe(
        &self,
        name: &str,
        items_json: &str,
        direction: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO starter_recipes (created_at, name, items_json, direction)              VALUES (?1, ?2, ?3, ?4)",
            params![Utc::now().to_rfc3339(), name, items_json, direction],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// #419 S4: saved recipes, most-recent-first. Raw rows — the caller
    /// parses `items_json` and skips what no longer parses.
    pub fn list_recipes(&self) -> Result<Vec<SavedRecipeRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, items_json, direction              FROM starter_recipes ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SavedRecipeRow {
                id: row.get(0)?,
                name: row.get(1)?,
                items_json: row.get(2)?,
                direction: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// #419 S4: forget a saved recipe. Unknown ids are a no-op — the row
    /// being gone IS the requested state.
    pub fn delete_recipe(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM starter_recipes WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// #419 S4: the newest log row for one source (recall reads the last
    /// begun opener from here — its STORED seed, never a re-hash).
    pub fn latest_exercise_for_source(
        &self,
        source: &str,
    ) -> Result<Option<ExerciseLogEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT source, label, spec_json, seed, difficulty, tonic, accuracy              FROM exercise_log WHERE source = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![source], |row| {
            Ok(ExerciseLogEntry {
                source: row.get(0)?,
                label: row.get(1)?,
                spec_json: row.get(2)?,
                seed: row.get::<_, i64>(3)? as u64,
                difficulty: row.get::<_, i64>(4)?.clamp(0, 255) as u8,
                tonic: row.get::<_, i64>(5)?.clamp(0, 255) as u8,
                accuracy: row.get(6)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    /// The default on-disk path for the sessions database.
    ///
    /// Follows the platform data-directory convention:
    /// - macOS: `~/Library/Application Support/ai-music-companion/sessions.db`
    /// - Linux: `$XDG_DATA_HOME/ai-music-companion/sessions.db`
    /// - Windows: `%APPDATA%\ai-music-companion\sessions.db`
    pub fn default_path() -> Result<PathBuf, StoreError> {
        let mut path = dirs::data_dir().ok_or(StoreError::NoDataDir)?;
        path.push("ai-music-companion");
        path.push("sessions.db");
        Ok(path)
    }
}

// ---------------------------------------------------------------------------
// ScoreStore
// ---------------------------------------------------------------------------

/// SQLite-backed store of loaded musical scores.
pub struct ScoreStore {
    conn: Connection,
}

impl ScoreStore {
    /// Open (or create) the shared database — uses the same connection
    /// as [`SessionStore`] for simplicity. Both stores operate on
    /// `sessions.db`, which contains both sessions and scores tables.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Open an ephemeral in-memory database. Used by tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Execute the schema migrations. Idempotent.
    fn migrate(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(SCHEMA)?;
        // Dedup-by-content (#385): databases created before `content_hash`
        // gain the column here, and pre-existing rows are backfilled so a
        // re-import of a score that predates the column still lands on its
        // old entry instead of minting a duplicate.
        add_column_if_missing(&self.conn, "scores", "content_hash", "TEXT")?;
        self.backfill_content_hashes()?;
        Ok(())
    }

    /// Hash any rows the migration left without a `content_hash`. Only ever
    /// touches NULL rows, so repeated opens are no-ops.
    fn backfill_content_hashes(&self) -> Result<(), StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, music_xml FROM scores WHERE content_hash IS NULL")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        for (id, music_xml) in rows {
            self.conn.execute(
                "UPDATE scores SET content_hash = ?1 WHERE id = ?2",
                params![score_content_hash(&music_xml), id],
            )?;
        }
        Ok(())
    }

    /// Import a score: parse it, validate it, persist it to the library.
    ///
    /// Returns the [`ScoreLibraryEntry`] that was stored.
    /// `music_xml` is the raw MusicXML string (parsed by the caller from file).
    /// `title`, `composer`, and other metadata may come from the parsed content
    /// or be inferred from the filename.
    ///
    /// Re-importing identical content is a no-op by design (#385): every
    /// format funnels its normalized MusicXML through here, and dropping the
    /// same file twice used to mint a new row each time. Same content + same
    /// part = the same score, so the existing entry is returned instead.
    /// Title matching is deliberately NOT used — different pieces can share a
    /// name; identical content can't differ.
    pub fn import(
        &self,
        title: String,
        composer: Option<String>,
        source_filename: String,
        music_xml: String,
        part_index: usize,
        duration_measures: usize,
    ) -> Result<ScoreLibraryEntry, StoreError> {
        let content_hash = score_content_hash(&music_xml);
        // Libraries that predate the hash column can already hold duplicates
        // (the bug this fixes), so the lookup is not unique; newest wins.
        // The music_xml equality check makes a 64-bit hash collision merge
        // impossible rather than astronomically unlikely — the hash is just
        // the cheap filter in front of the blob compare.
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM scores \
                 WHERE content_hash = ?1 AND part_index = ?2 AND music_xml = ?3 \
                 ORDER BY added_at DESC LIMIT 1",
                params![content_hash, part_index as i64, music_xml],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id_str) = existing {
            let id: ScoreId = id_str.parse().map_err(|e: uuid::Error| {
                StoreError::CorruptRow(format!("invalid score id {id_str}: {e}"))
            })?;
            return self.get(id);
        }

        let id = ScoreId::new();
        let now = Utc::now();

        self.conn.execute(
            "INSERT INTO scores \
             (id, title, composer, source_filename, added_at, part_index, duration_measures, music_xml, content_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id.as_str(),
                title,
                composer,
                source_filename.clone(),
                now.to_rfc3339(),
                part_index as i64,
                duration_measures as i64,
                music_xml.clone(),
                content_hash,
            ],
        )?;

        Ok(ScoreLibraryEntry {
            id,
            title,
            composer,
            source_filename,
            added_at: now,
            last_practiced_at: None,
            part_index,
            duration_measures,
            music_xml,
        })
    }

    /// List all scores in the library, ordered by last_practiced_at desc,
    /// then added_at desc. Returns both metadata and the raw MusicXML.
    pub fn list(&self) -> Result<Vec<ScoreLibraryEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, composer, source_filename, added_at, last_practiced_at, \
                    part_index, duration_measures, music_xml \
             FROM scores \
             ORDER BY last_practiced_at DESC NULLS LAST, added_at DESC",
        )?;

        let scores = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in scores {
            let (
                id_str,
                title,
                composer,
                source_filename,
                added_at_str,
                last_practiced_at_str,
                part_index,
                duration_measures,
                music_xml,
            ) = row?;

            let id: ScoreId = id_str.parse().map_err(|e: uuid::Error| {
                StoreError::CorruptRow(format!("invalid score id {id_str}: {e}"))
            })?;

            let added_at = DateTime::parse_from_rfc3339(&added_at_str)
                .map_err(|e| {
                    StoreError::CorruptRow(format!("invalid RFC3339 added_at {added_at_str}: {e}"))
                })?
                .with_timezone(&Utc);

            let last_practiced_at = last_practiced_at_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            });

            result.push(ScoreLibraryEntry {
                id,
                title,
                composer,
                source_filename,
                added_at,
                last_practiced_at,
                part_index: part_index as usize,
                duration_measures: duration_measures as usize,
                music_xml,
            });
        }

        Ok(result)
    }

    /// Load a single score by id. Returns the full entry including MusicXML.
    pub fn get(&self, id: ScoreId) -> Result<ScoreLibraryEntry, StoreError> {
        let id_str = id.as_str();
        let row: Option<ScoreRow> = self
            .conn
            .query_row(
                "SELECT id, title, composer, source_filename, added_at, last_practiced_at, \
                        part_index, duration_measures, music_xml FROM scores WHERE id = ?1",
                params![id_str],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .optional()?;

        match row {
            Some((
                id_str,
                title,
                composer,
                source_filename,
                added_at_str,
                last_practiced_at_str,
                part_index,
                duration_measures,
                music_xml,
            )) => {
                let added_at = DateTime::parse_from_rfc3339(&added_at_str)
                    .map_err(|e| {
                        StoreError::CorruptRow(format!(
                            "invalid RFC3339 added_at {added_at_str}: {e}"
                        ))
                    })?
                    .with_timezone(&Utc);

                let last_practiced_at = last_practiced_at_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });

                Ok(ScoreLibraryEntry {
                    id: id_str.parse().map_err(|e: uuid::Error| {
                        StoreError::CorruptRow(format!("invalid score id {id_str}: {e}"))
                    })?,
                    title,
                    composer,
                    source_filename,
                    added_at,
                    last_practiced_at,
                    part_index: part_index as usize,
                    duration_measures: duration_measures as usize,
                    music_xml,
                })
            }
            None => Err(StoreError::NotFound(format!("score {id_str}"))),
        }
    }

    /// Delete a score and its MusicXML from the library.
    pub fn delete(&self, id: ScoreId) -> Result<(), StoreError> {
        let id_str = id.as_str();
        self.conn
            .execute("DELETE FROM scores WHERE id = ?1", params![id_str])?;
        Ok(())
    }

    /// Update the last_practiced_at timestamp for a score.
    pub fn update_last_practiced(&self, id: ScoreId) -> Result<(), StoreError> {
        let id_str = id.as_str();
        let now = Utc::now();
        self.conn.execute(
            "UPDATE scores SET last_practiced_at = ?1 WHERE id = ?2",
            params![now.to_rfc3339(), id_str],
        )?;
        Ok(())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    /// Exercise log roundtrip: appends read back oldest-first with the graded
    /// vs ungraded distinction intact; errors surface (not swallowed here).
    #[test]
    fn exercise_log_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let s = SessionStore::open(&dir.path().join("t.db")).unwrap();
        let mut e = ExerciseLogEntry {
            source: "lesson".to_owned(),
            label: "C Major · up-down".to_owned(),
            spec_json: "{}".to_owned(),
            seed: u64::MAX, // the i64 storage roundtrip must survive extremes
            difficulty: 3,
            tonic: 7,
            accuracy: Some(0.85),
        };
        s.log_exercise(&e).unwrap();
        e.source = "explore".to_owned();
        e.accuracy = None;
        s.log_exercise(&e).unwrap();
        let back = s.list_exercise_log().unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].source, "lesson");
        assert_eq!(back[0].accuracy, Some(0.85));
        assert_eq!(back[0].seed, u64::MAX);
        assert_eq!(back[1].accuracy, None);
    }

    /// #453 S1 AC6: the timed reader returns the SAME rows as the untimed
    /// one plus an RFC3339-parseable write stamp — the history analyzer's
    /// time axis. Fails if the timed SELECT drops/reorders columns or the
    /// store starts writing stamps chrono can't read back.
    #[test]
    fn timed_exercise_log_carries_parseable_stamps() {
        let dir = tempfile::tempdir().unwrap();
        let s = SessionStore::open(&dir.path().join("t.db")).unwrap();
        let e = ExerciseLogEntry {
            source: "lesson".to_owned(),
            label: "C Major · up-down".to_owned(),
            spec_json: "{}".to_owned(),
            seed: u64::MAX,
            difficulty: 3,
            tonic: 7,
            accuracy: Some(0.85),
        };
        s.log_exercise(&e).unwrap();
        let timed = s.list_exercise_log_timed().unwrap();
        assert_eq!(timed.len(), 1);
        assert_eq!(timed[0].entry, s.list_exercise_log().unwrap()[0]);
        let stamp = chrono::DateTime::parse_from_rfc3339(&timed[0].logged_at)
            .expect("store-written logged_at parses as RFC3339");
        let age = chrono::Utc::now().signed_duration_since(stamp);
        assert!(
            age.num_seconds().abs() < 3600,
            "stamp is the write time, not a constant: {}",
            timed[0].logged_at
        );
    }

    fn recap_with(instrument: &str, duration: f64, phrase_count: usize) -> SessionRecap {
        SessionRecap {
            score_summary: None,
            overall_assessment: format!("Assessment for {instrument}"),
            strengths: vec![
                "Stable long tones.".to_owned(),
                "Expressive phrasing.".to_owned(),
            ],
            areas_to_improve: vec![
                "Intonation in upper register.".to_owned(),
                "Consistent articulation.".to_owned(),
            ],
            next_session_suggestions: vec![
                "Warm up with a chromatic scale.".to_owned(),
                "Practice with a drone.".to_owned(),
            ],
            duration_secs: duration,
            phrase_count,
            instrument: instrument.to_owned(),
            fingerprint: None,
            flavour: None,
            idiom_notes: Vec::new(),
            connections: Vec::new(),
        }
    }

    #[test]
    fn in_memory_store_opens_clean() {
        let store = SessionStore::in_memory().unwrap();
        // A fresh store has no rows to list.
        let recent = store.list_recent(10).unwrap();
        assert!(
            recent.is_empty(),
            "fresh in-memory store must have zero sessions, got {}",
            recent.len()
        );
    }

    #[test]
    fn save_then_load_roundtrip() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let started = Utc::now();
        let ended = started + Duration::seconds(42);
        let recap = recap_with("trumpet", 42.0, 5);

        store.save(id, started, ended, &recap).unwrap();
        let loaded = store.load(id).unwrap();

        // Persisted timestamps round-trip too — this is the whole point of
        // returning StoredSession rather than just the recap body.
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.started_at, started);
        assert_eq!(loaded.ended_at, ended);

        // Per-field equality, not whole-object JSON string comparison,
        // so we catch any silent coercion (e.g. f64 rounding).
        let r = &loaded.recap;
        assert_eq!(r.overall_assessment, recap.overall_assessment);
        assert_eq!(r.strengths, recap.strengths);
        assert_eq!(r.areas_to_improve, recap.areas_to_improve);
        assert_eq!(r.next_session_suggestions, recap.next_session_suggestions);
        assert_eq!(r.phrase_count, recap.phrase_count);
        assert_eq!(r.instrument, recap.instrument);
        assert!(
            (r.duration_secs - recap.duration_secs).abs() < 1e-9,
            "duration roundtrip mismatch: {} vs {}",
            r.duration_secs,
            recap.duration_secs
        );
    }

    #[test]
    fn fingerprint_persists_and_legacy_recaps_load_as_none() {
        use crate::fingerprint::MusicalFingerprint;

        let store = SessionStore::in_memory().unwrap();

        // A recap carrying a musical fingerprint round-trips through the
        // recap_json column.
        let id = SessionId::new();
        let now = Utc::now();
        let recap = SessionRecap {
            score_summary: None,
            fingerprint: Some(MusicalFingerprint {
                tone: Some(tone::ToneDescriptor {
                    brightness: 0.6,
                    warmth: 0.5,
                    air_noise: 0.2,
                    core_clarity: 0.8,
                    vibrato_quality: 0.55,
                }),
                key: None,
                key_claim: None,
                intonation: None,
                groove: None,
            }),
            ..recap_with("voice", 60.0, 9)
        };
        store.save(id, now, now, &recap).unwrap();
        let loaded = store.load(id).unwrap();
        assert_eq!(loaded.recap.fingerprint, recap.fingerprint);

        // A recap_json saved before `fingerprint` existed (field absent) must
        // still deserialize, defaulting to None.
        let legacy: SessionRecap = serde_json::from_str(
            r#"{"overall_assessment":"ok","strengths":[],"areas_to_improve":[],
                "next_session_suggestions":[],"duration_secs":1.0,"phrase_count":1,
                "instrument":"flute"}"#,
        )
        .expect("legacy recap deserializes");
        assert!(legacy.fingerprint.is_none());
        // The same legacy blob predates `connections` too — defaults to empty.
        assert!(legacy.connections.is_empty());
    }

    #[test]
    fn connections_persist_through_recap_json() {
        // Grounded cross-genre connections must round-trip through the
        // recap_json column like any other recap text.
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        let recap = SessionRecap {
            score_summary: None,
            connections: vec![
                "your laid-back time has the same pocket as a lot of the soul you love".to_owned(),
            ],
            ..recap_with("trumpet", 60.0, 4)
        };
        store.save(id, now, now, &recap).unwrap();
        let loaded = store.load(id).unwrap();
        assert_eq!(loaded.recap.connections, recap.connections);
    }

    #[test]
    fn load_missing_returns_not_found() {
        let store = SessionStore::in_memory().unwrap();
        let unknown = SessionId::new();
        let err = store.load(unknown).unwrap_err();
        match err {
            StoreError::NotFound(id) => assert_eq!(id, unknown.as_str()),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn list_recent_orders_by_started_at_desc() {
        let store = SessionStore::in_memory().unwrap();

        // Three sessions, each 1 hour apart; oldest saved first.
        let now = Utc::now();
        let id_oldest = SessionId::new();
        let id_middle = SessionId::new();
        let id_newest = SessionId::new();

        store
            .save(
                id_oldest,
                now - Duration::hours(2),
                now - Duration::hours(2) + Duration::seconds(60),
                &recap_with("trumpet", 60.0, 1),
            )
            .unwrap();
        store
            .save(
                id_middle,
                now - Duration::hours(1),
                now - Duration::hours(1) + Duration::seconds(60),
                &recap_with("violin", 60.0, 2),
            )
            .unwrap();
        store
            .save(
                id_newest,
                now,
                now + Duration::seconds(60),
                &recap_with("voice", 60.0, 3),
            )
            .unwrap();

        let recent = store.list_recent(10).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].id, id_newest, "newest must come first");
        assert_eq!(recent[1].id, id_middle);
        assert_eq!(recent[2].id, id_oldest);

        // Monotonically non-increasing started_at.
        assert!(recent[0].started_at >= recent[1].started_at);
        assert!(recent[1].started_at >= recent[2].started_at);

        // Instrument column round-trips.
        assert_eq!(recent[0].instrument, "voice");
        assert_eq!(recent[1].instrument, "violin");
        assert_eq!(recent[2].instrument, "trumpet");

        // Phrase count survives the i64 <-> usize round trip.
        assert_eq!(recent[0].phrase_count, 3);
        assert_eq!(recent[1].phrase_count, 2);
        assert_eq!(recent[2].phrase_count, 1);
    }

    #[test]
    fn list_recent_respects_limit() {
        let store = SessionStore::in_memory().unwrap();
        let base = Utc::now();
        for i in 0_usize..5 {
            let started = base - Duration::minutes(i64::try_from(i).unwrap());
            store
                .save(
                    SessionId::new(),
                    started,
                    started + Duration::seconds(10),
                    &recap_with("trumpet", 10.0, i),
                )
                .unwrap();
        }

        let two = store.list_recent(2).unwrap();
        assert_eq!(two.len(), 2, "limit=2 must return exactly 2 rows");
    }

    #[test]
    fn save_preserves_all_recap_fields() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let recap = SessionRecap {
            score_summary: None,
            overall_assessment: "Solid session overall.".to_owned(),
            strengths: vec!["A".to_owned(), "B".to_owned(), "C".to_owned()],
            areas_to_improve: vec!["X".to_owned(), "Y".to_owned()],
            next_session_suggestions: vec!["One".to_owned(), "Two".to_owned(), "Three".to_owned()],
            duration_secs: 123.456,
            phrase_count: 17,
            instrument: "clarinet".to_owned(),
            fingerprint: None,
            flavour: None,
            idiom_notes: Vec::new(),
            connections: Vec::new(),
        };
        let now = Utc::now();
        store
            .save(id, now, now + Duration::seconds(124), &recap)
            .unwrap();

        let loaded = store.load(id).unwrap();
        let r = &loaded.recap;
        assert_eq!(
            r.strengths.len(),
            3,
            "strengths vector length must be preserved"
        );
        assert_eq!(r.strengths[0], "A");
        assert_eq!(r.strengths[1], "B");
        assert_eq!(r.strengths[2], "C");
        assert_eq!(r.areas_to_improve.len(), 2);
        assert_eq!(r.areas_to_improve[0], "X");
        assert_eq!(r.areas_to_improve[1], "Y");
        assert_eq!(r.next_session_suggestions.len(), 3);
        assert_eq!(r.next_session_suggestions[2], "Three");
        assert_eq!(r.phrase_count, 17);
    }

    #[test]
    fn default_path_uses_data_dir() {
        let path = SessionStore::default_path().expect("data_dir should exist on this platform");
        let as_string = path.to_string_lossy().to_string();
        assert!(
            as_string.ends_with("ai-music-companion/sessions.db")
                || as_string.ends_with("ai-music-companion\\sessions.db"),
            "default_path must point into the ai-music-companion data dir, got {as_string}"
        );
    }

    #[test]
    fn duration_secs_preserves_float_precision() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        let recap = recap_with("trumpet", 123.456, 4);
        store
            .save(id, now, now + Duration::seconds(124), &recap)
            .unwrap();

        let loaded = store.load(id).unwrap();
        assert!(
            (loaded.recap.duration_secs - 123.456).abs() < 1e-9,
            "duration_secs must preserve f64 precision, got {}",
            loaded.recap.duration_secs
        );
    }

    #[test]
    fn migration_is_idempotent_on_file_backed_store() {
        // Use a tempfile-style path in the system temp dir.
        let dir = std::env::temp_dir();
        let file = dir.join(format!(
            "ai-music-companion-test-{}.db",
            SessionId::new().as_str()
        ));

        // Open twice in sequence: the second open must not error.
        let first = SessionStore::open(&file).unwrap();
        drop(first);
        let second = SessionStore::open(&file).unwrap();
        // And we can still list — proving the schema is healthy after
        // two migrate() runs against the same file.
        let _ = second.list_recent(1).unwrap();

        // Clean up.
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn save_overwrites_existing_session_by_id() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        let first = recap_with("trumpet", 10.0, 1);
        let second = recap_with("violin", 20.0, 2);
        store
            .save(id, now, now + Duration::seconds(10), &first)
            .unwrap();
        store
            .save(id, now, now + Duration::seconds(20), &second)
            .unwrap();

        let loaded = store.load(id).unwrap();
        assert_eq!(
            loaded.recap.instrument, "violin",
            "second save with same id must overwrite the first row"
        );
        assert!((loaded.recap.duration_secs - 20.0).abs() < 1e-9);
    }

    #[test]
    fn summary_started_at_roundtrips_rfc3339() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        // A non-trivial timestamp to exercise formatting.
        let started = DateTime::parse_from_rfc3339("2026-01-15T12:34:56.789Z")
            .unwrap()
            .with_timezone(&Utc);
        let recap = recap_with("trumpet", 30.0, 2);
        store
            .save(id, started, started + Duration::seconds(30), &recap)
            .unwrap();

        let recent = store.list_recent(1).unwrap();
        assert_eq!(recent.len(), 1);
        // Compare at millisecond precision — that is what RFC3339 with
        // fractional seconds preserves via chrono round-trip.
        let delta = (recent[0].started_at - started).num_milliseconds().abs();
        assert!(
            delta <= 1,
            "started_at must roundtrip within 1ms, got delta {delta}ms"
        );
    }

    #[test]
    fn save_derives_duration_from_timestamps_not_recap_blob() {
        // Regression: `save()` used to copy `recap.duration_secs` into the
        // summary column, which let the row drift internally — e.g. a
        // recap with duration=0.5 persisted against a 30-second span of
        // (ended_at - started_at) would make `list_recent()` report 0.5
        // instead of 30. The column must match the timestamps.
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let started = Utc::now();
        let ended = started + Duration::seconds(30);

        // Recap claims something wildly different from the real span.
        let lying_recap = recap_with("trumpet", 0.5, 1);
        store.save(id, started, ended, &lying_recap).unwrap();

        let summary = store
            .list_recent(1)
            .unwrap()
            .into_iter()
            .next()
            .expect("one session");
        assert!(
            (summary.duration_secs - 30.0).abs() < 0.01,
            "summary.duration_secs must be timestamp-derived (≈30s), got {}",
            summary.duration_secs,
        );

        // The recap blob itself is preserved unchanged so opaque LLM
        // text survives, but the indexable column is authoritative.
        let loaded = store.load(id).unwrap();
        assert!((loaded.recap.duration_secs - 0.5).abs() < 1e-9);
    }

    #[test]
    fn list_by_instrument_filters_correctly() {
        let store = SessionStore::in_memory().unwrap();
        let now = Utc::now();

        let trumpet_id = SessionId::new();
        let violin_id = SessionId::new();
        let trumpet_id_2 = SessionId::new();

        store
            .save(
                trumpet_id,
                now - Duration::minutes(3),
                now - Duration::minutes(2),
                &recap_with("trumpet", 60.0, 1),
            )
            .unwrap();
        store
            .save(
                violin_id,
                now - Duration::minutes(2),
                now - Duration::minutes(1),
                &recap_with("violin", 60.0, 1),
            )
            .unwrap();
        store
            .save(
                trumpet_id_2,
                now,
                now + Duration::minutes(1),
                &recap_with("trumpet", 60.0, 1),
            )
            .unwrap();

        let trumpet_sessions = store.list_by_instrument(Some("trumpet")).unwrap();
        assert_eq!(trumpet_sessions.len(), 2);
        assert_eq!(trumpet_sessions[0].instrument, "trumpet");
        assert_eq!(trumpet_sessions[1].instrument, "trumpet");

        let violin_sessions = store.list_by_instrument(Some("violin")).unwrap();
        assert_eq!(violin_sessions.len(), 1);
        assert_eq!(violin_sessions[0].instrument, "violin");

        let all_sessions = store.list_by_instrument(None).unwrap();
        assert_eq!(all_sessions.len(), 3);
    }

    #[test]
    fn list_by_date_range_filters_correctly() {
        let store = SessionStore::in_memory().unwrap();
        let base = Utc::now();
        let day1 = base - Duration::days(2);
        let day2 = base - Duration::days(1);
        let day3 = base;

        store
            .save(
                SessionId::new(),
                day1,
                day1 + Duration::minutes(10),
                &recap_with("trumpet", 600.0, 1),
            )
            .unwrap();
        store
            .save(
                SessionId::new(),
                day2,
                day2 + Duration::minutes(10),
                &recap_with("trumpet", 600.0, 1),
            )
            .unwrap();
        store
            .save(
                SessionId::new(),
                day3,
                day3 + Duration::minutes(10),
                &recap_with("trumpet", 600.0, 1),
            )
            .unwrap();

        let all = store.list_by_date_range(None, None).unwrap();
        assert_eq!(all.len(), 3);

        let recent_two_days = store.list_by_date_range(Some(day2), None).unwrap();
        assert_eq!(recent_two_days.len(), 2);

        let one_day_only = store.list_by_date_range(Some(day2), Some(day3)).unwrap();
        assert_eq!(one_day_only.len(), 1);
    }

    #[test]
    fn count_sessions_returns_correct_total() {
        let store = SessionStore::in_memory().unwrap();
        assert_eq!(store.count_sessions().unwrap(), 0);

        let now = Utc::now();
        for i in 0..5 {
            store
                .save(
                    SessionId::new(),
                    now - Duration::minutes(i as i64),
                    now - Duration::minutes((i as i64) - 1),
                    &recap_with("trumpet", 60.0, 1),
                )
                .unwrap();
        }

        assert_eq!(store.count_sessions().unwrap(), 5);
    }

    #[test]
    fn migration_adds_session_debug_columns_to_legacy_db() {
        // A DB created before the debug columns existed: the legacy `sessions`
        // shape, user_version 0, and a row already present.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY, instrument TEXT NOT NULL,
                 started_at TEXT NOT NULL, ended_at TEXT NOT NULL,
                 duration_secs REAL NOT NULL, phrase_count INTEGER NOT NULL,
                 recap_json TEXT NOT NULL);
             INSERT INTO sessions VALUES ('s1','Trumpet','t0','t1',60.0,3,'{}');",
        )
        .unwrap();
        let store = SessionStore { conn };

        store
            .migrate()
            .expect("migration must succeed on a legacy DB");

        let cols: Vec<String> = store
            .conn
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for c in ["score_id", "app_version", "practice_mode"] {
            assert!(cols.iter().any(|n| n == c), "column {c} must be added");
        }
        // Existing data survived the ALTERs untouched.
        let instrument: String = store
            .conn
            .query_row("SELECT instrument FROM sessions WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(instrument, "Trumpet");
        // Idempotent: a second migrate is a no-op (user_version already 1).
        store.migrate().expect("second migrate must be a no-op");
    }

    #[test]
    fn record_session_meta_round_trips() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        store
            .save(
                id,
                now,
                now + Duration::seconds(30),
                &recap_with("trumpet", 30.0, 2),
            )
            .unwrap();

        assert_eq!(store.session_meta(id).unwrap(), SessionMeta::default());

        // A score must exist for the score_id FK to be satisfiable.
        store
            .conn
            .execute_batch(
                "INSERT INTO scores (id, title, source_filename, added_at, music_xml) \
                 VALUES ('score-123', 't', 'f', 't0', '<x/>');",
            )
            .unwrap();
        store
            .record_session_meta(id, Some("9.9.9"), Some("Practice"), Some("score-123"))
            .unwrap();
        let meta = store.session_meta(id).unwrap();
        assert_eq!(meta.app_version.as_deref(), Some("9.9.9"));
        assert_eq!(meta.practice_mode.as_deref(), Some("Practice"));
        assert_eq!(meta.score_id.as_deref(), Some("score-123"));
    }

    #[test]
    fn total_duration_secs_sums_all_sessions() {
        let store = SessionStore::in_memory().unwrap();
        assert_eq!(store.total_duration_secs().unwrap(), 0.0);

        let now = Utc::now();
        store
            .save(
                SessionId::new(),
                now,
                now + Duration::seconds(30),
                &recap_with("trumpet", 30.0, 1),
            )
            .unwrap();
        store
            .save(
                SessionId::new(),
                now + Duration::seconds(30),
                now + Duration::seconds(90),
                &recap_with("trumpet", 60.0, 1),
            )
            .unwrap();

        let total = store.total_duration_secs().unwrap();
        assert!((total - 90.0).abs() < 0.1);
    }

    #[test]
    fn list_recent_rejects_corrupt_phrase_count() {
        // Regression: a negative `phrase_count` row (only reachable via an
        // externally-mutated DB) used to silently coerce to 0, presenting
        // a fabricated zero-phrase summary. Now it escalates to CorruptRow
        // consistent with every other malformed-field path in the module.
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        store
            .save(
                id,
                now,
                now + Duration::seconds(10),
                &recap_with("x", 10.0, 0),
            )
            .unwrap();

        // Corrupt the row directly.
        store
            .conn
            .execute(
                "UPDATE sessions SET phrase_count = -1 WHERE id = ?1",
                params![id.as_str()],
            )
            .unwrap();

        let err = store.list_recent(10).unwrap_err();
        match err {
            StoreError::CorruptRow(msg) => assert!(
                msg.contains("phrase_count"),
                "CorruptRow message should mention phrase_count, got: {msg}"
            ),
            other => panic!("expected CorruptRow, got {other:?}"),
        }
    }

    // All three list_* methods share a single decode_summary_row helper —
    // these two tests lock the invariant that the other two paths escalate
    // corrupt data the same way `list_recent` does.

    #[test]
    fn list_by_instrument_rejects_corrupt_phrase_count() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        store
            .save(
                id,
                now,
                now + Duration::seconds(10),
                &recap_with("trumpet", 10.0, 0),
            )
            .unwrap();

        store
            .conn
            .execute(
                "UPDATE sessions SET phrase_count = -1 WHERE id = ?1",
                params![id.as_str()],
            )
            .unwrap();

        let err = store.list_by_instrument(Some("trumpet")).unwrap_err();
        assert!(
            matches!(err, StoreError::CorruptRow(ref msg) if msg.contains("phrase_count")),
            "expected CorruptRow(phrase_count), got {err:?}"
        );
    }

    #[test]
    fn list_by_date_range_rejects_corrupt_phrase_count() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        store
            .save(
                id,
                now,
                now + Duration::seconds(10),
                &recap_with("trumpet", 10.0, 0),
            )
            .unwrap();

        store
            .conn
            .execute(
                "UPDATE sessions SET phrase_count = -1 WHERE id = ?1",
                params![id.as_str()],
            )
            .unwrap();

        let err = store.list_by_date_range(None, None).unwrap_err();
        assert!(
            matches!(err, StoreError::CorruptRow(ref msg) if msg.contains("phrase_count")),
            "expected CorruptRow(phrase_count), got {err:?}"
        );
    }

    // Regression for the CTO-audit finding: `count_sessions` was returning
    // `Ok(0)` on a negative SQLite count (silent data-corruption concealment).
    // A bare `SELECT COUNT(*)` can't actually return a negative value in
    // normal operation, so we exercise the real `decode_session_count`
    // helper that `count_sessions` delegates to — that way the production
    // code path (not a duplicated `usize::try_from`) is covered and a
    // future refactor that weakens the guard will fail this test.
    #[test]
    fn count_sessions_escalates_negative_count_to_corrupt_row() {
        let store = SessionStore::in_memory().unwrap();
        // Happy path: real call through the real query still works.
        assert_eq!(store.count_sessions().unwrap(), 0);
        assert_eq!(decode_session_count(0).unwrap(), 0);
        assert_eq!(decode_session_count(5).unwrap(), 5);

        // Negative signals corruption — must escalate, not silently clamp.
        let err = decode_session_count(-1);
        assert!(
            matches!(err, Err(StoreError::CorruptRow(ref msg)) if msg.contains("negative")),
            "expected CorruptRow(negative...), got {err:?}"
        );
    }

    // =========================================================================
    // TasteProfile tests
    // =========================================================================

    fn sample_taste_profile() -> TasteProfile {
        TasteProfile {
            genres: vec!["hip-hop".to_owned(), "gospel".to_owned()],
            artists: vec!["Kendrick Lamar".to_owned(), "Hans Zimmer".to_owned()],
            goals: vec!["audition prep".to_owned()],
            experience: ExperienceLevel::Intermediate,
            is_under_13: false,
        }
    }

    /// #252 F2: the learner model persists — cold start is `None`, an upsert
    /// roundtrips the full blob (collection entries included), and a second
    /// upsert overwrites in place. Fails if the table/serde plumbing drops data.
    #[test]
    fn learner_model_roundtrips_and_overwrites_in_place() {
        let store = SessionStore::in_memory().unwrap();
        assert!(
            store
                .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
                .unwrap()
                .is_none(),
            "cold start must report no learner model"
        );

        let m1 = crate::learner::apply_reveal(
            &crate::learner::LearnerModel::default(),
            "G Dorian",
            "Miles Davis — \"So What\"",
            100,
        );
        store
            .upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &m1)
            .unwrap();
        let got = store
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .expect("model exists after upsert");
        assert_eq!(got, m1);
        assert_eq!(got.collection_size(), 1);

        // Overwrite in place — one row per user, no duplicates.
        let m2 = crate::learner::apply_reveal(&m1, "C Major", "Beethoven — \"Ode to Joy\"", 200);
        store
            .upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &m2)
            .unwrap();
        let got2 = store
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .unwrap();
        assert_eq!(got2.collection_size(), 2);
        assert_eq!(got2, m2);
    }

    #[test]
    fn taste_profile_absent_before_capture() {
        let store = SessionStore::in_memory().unwrap();
        let got = store
            .get_taste_profile(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap();
        assert!(
            got.is_none(),
            "cold start must report no taste profile, not an error or default row"
        );
    }

    #[test]
    fn taste_profile_upsert_then_get_roundtrips() {
        let store = SessionStore::in_memory().unwrap();
        let profile = sample_taste_profile();
        store
            .upsert_taste_profile(LOCAL_TASTE_PROFILE_USER_ID, &profile)
            .unwrap();

        let got = store
            .get_taste_profile(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .expect("profile should exist after upsert");
        // Per-field equality so any silent coercion is caught.
        assert_eq!(got.genres, profile.genres);
        assert_eq!(got.artists, profile.artists);
        assert_eq!(got.goals, profile.goals);
        assert_eq!(got.experience, ExperienceLevel::Intermediate);
        assert!(!got.is_under_13);
        assert_eq!(got, profile);
    }

    #[test]
    fn taste_profile_upsert_overwrites_in_place() {
        // One row per user: a second upsert edits, never appends a duplicate.
        let store = SessionStore::in_memory().unwrap();
        store
            .upsert_taste_profile(LOCAL_TASTE_PROFILE_USER_ID, &sample_taste_profile())
            .unwrap();

        let edited = TasteProfile {
            genres: vec!["jazz".to_owned()],
            experience: ExperienceLevel::Advanced,
            is_under_13: true,
            ..TasteProfile::default()
        };
        store
            .upsert_taste_profile(LOCAL_TASTE_PROFILE_USER_ID, &edited)
            .unwrap();

        let got = store
            .get_taste_profile(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .unwrap();
        assert_eq!(got, edited, "second upsert must replace the prior profile");
        assert_eq!(got.experience, ExperienceLevel::Advanced);
        assert!(got.is_under_13);

        // Still exactly one row for this user.
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM taste_profile WHERE user_id = ?1",
                params![LOCAL_TASTE_PROFILE_USER_ID],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "upsert must not create a duplicate row");
    }

    #[test]
    fn taste_profiles_are_isolated_per_user() {
        // The local row and a synced user's row are independent keys.
        let store = SessionStore::in_memory().unwrap();
        let local = sample_taste_profile();
        let remote = TasteProfile {
            genres: vec!["classical".to_owned()],
            ..TasteProfile::default()
        };
        store
            .upsert_taste_profile(LOCAL_TASTE_PROFILE_USER_ID, &local)
            .unwrap();
        store
            .upsert_taste_profile("user-uuid-123", &remote)
            .unwrap();

        assert_eq!(
            store
                .get_taste_profile(LOCAL_TASTE_PROFILE_USER_ID)
                .unwrap()
                .unwrap(),
            local
        );
        assert_eq!(
            store.get_taste_profile("user-uuid-123").unwrap().unwrap(),
            remote
        );
    }

    #[test]
    fn taste_profile_legacy_json_defaults_missing_fields() {
        // A profile_json saved before a field existed must still deserialise,
        // defaulting the missing field — the forward-compat contract.
        let legacy: TasteProfile = serde_json::from_str(r#"{"genres":["funk"]}"#)
            .expect("legacy taste profile deserialises");
        assert_eq!(legacy.genres, vec!["funk".to_owned()]);
        assert!(legacy.artists.is_empty());
        assert!(legacy.goals.is_empty());
        assert_eq!(legacy.experience, ExperienceLevel::Beginner);
        assert!(!legacy.is_under_13);

        // Fully empty object loads to all-defaults.
        let empty: TasteProfile = serde_json::from_str("{}").expect("empty object loads");
        assert_eq!(empty, TasteProfile::default());
    }

    #[test]
    fn experience_level_serializes_snake_case() {
        // The wire form must match the Supabase CHECK constraint values.
        assert_eq!(
            serde_json::to_string(&ExperienceLevel::Beginner).unwrap(),
            "\"beginner\""
        );
        assert_eq!(ExperienceLevel::Advanced.as_str(), "advanced");
        let back: ExperienceLevel = serde_json::from_str("\"intermediate\"").unwrap();
        assert_eq!(back, ExperienceLevel::Intermediate);
    }

    // =========================================================================
    // ScoreStore tests
    // =========================================================================

    #[test]
    fn score_store_in_memory_opens_clean() {
        let store = ScoreStore::in_memory().unwrap();
        let scores = store.list().unwrap();
        assert_eq!(scores.len(), 0, "fresh store has no scores");
    }

    #[test]
    fn import_score_persists_and_retrieves() {
        let store = ScoreStore::in_memory().unwrap();
        let music_xml = r#"<?xml version="1.0" encoding="UTF-8"?><score-partwise/>"#.to_string();

        let entry = store
            .import(
                "Haydn Trumpet Concerto".to_string(),
                Some("Joseph Haydn".to_string()),
                "haydn-trumpet.musicxml".to_string(),
                music_xml.clone(),
                0,
                30,
            )
            .unwrap();

        assert_eq!(entry.title, "Haydn Trumpet Concerto");
        assert_eq!(entry.composer, Some("Joseph Haydn".to_string()));
        assert_eq!(entry.part_index, 0);
        assert_eq!(entry.duration_measures, 30);
        assert_eq!(entry.last_practiced_at, None);

        let retrieved = store.get(entry.id).unwrap();
        assert_eq!(retrieved.id, entry.id);
        assert_eq!(retrieved.title, entry.title);
        assert_eq!(retrieved.music_xml, music_xml);
    }

    #[test]
    fn list_scores_ordered_by_last_practiced() {
        let store = ScoreStore::in_memory().unwrap();
        // Distinct content per score — identical content would (correctly)
        // dedup to a single entry (#385), and this test is about ordering.
        let xml1 =
            "<?xml version=\"1.0\"?><score-partwise><!-- one --></score-partwise>".to_string();
        let xml2 =
            "<?xml version=\"1.0\"?><score-partwise><!-- two --></score-partwise>".to_string();

        let id1 = store
            .import(
                "Score 1".to_string(),
                None,
                "s1.musicxml".to_string(),
                xml1,
                0,
                10,
            )
            .unwrap()
            .id;
        let id2 = store
            .import(
                "Score 2".to_string(),
                None,
                "s2.musicxml".to_string(),
                xml2,
                0,
                20,
            )
            .unwrap()
            .id;

        // Mark id2 as recently practiced.
        store.update_last_practiced(id2).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, id2, "recently practiced should come first");
        assert_eq!(list[1].id, id1, "unpracticed should come second");
        assert!(list[0].last_practiced_at.is_some());
        assert!(list[1].last_practiced_at.is_none());
    }

    #[test]
    fn delete_score_removes_from_library() {
        let store = ScoreStore::in_memory().unwrap();
        let xml = "<?xml version=\"1.0\"?><score-partwise/>".to_string();

        let id = store
            .import(
                "Test Score".to_string(),
                None,
                "test.musicxml".to_string(),
                xml,
                0,
                10,
            )
            .unwrap()
            .id;

        assert_eq!(store.list().unwrap().len(), 1);
        store.delete(id).unwrap();
        assert_eq!(store.list().unwrap().len(), 0);
    }

    #[test]
    fn get_missing_score_returns_not_found() {
        let store = ScoreStore::in_memory().unwrap();
        let unknown = ScoreId::new();
        let err = store.get(unknown).unwrap_err();
        match err {
            StoreError::NotFound(msg) => assert!(msg.contains(&unknown.as_str())),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn update_last_practiced_timestamp_works() {
        let store = ScoreStore::in_memory().unwrap();
        let xml = "<?xml version=\"1.0\"?><score-partwise/>".to_string();

        let id = store
            .import(
                "Test".to_string(),
                None,
                "test.musicxml".to_string(),
                xml,
                0,
                10,
            )
            .unwrap()
            .id;

        let before = store.get(id).unwrap();
        assert_eq!(before.last_practiced_at, None);

        store.update_last_practiced(id).unwrap();

        let after = store.get(id).unwrap();
        assert!(after.last_practiced_at.is_some());
        assert!(after.last_practiced_at.unwrap() > before.added_at);
    }

    /// The hashes persist in the DB, so the function's contract is byte-for-
    /// byte stability across releases — pin it to the published FNV-1a 64
    /// vectors. If this test goes red, the change would silently orphan every
    /// stored hash and bring the #385 duplicates back for existing libraries.
    #[test]
    fn score_content_hash_is_pinned_to_fnv1a64_vectors() {
        assert_eq!(score_content_hash(""), "cbf29ce484222325");
        assert_eq!(score_content_hash("a"), "af63dc4c8601ec8c");
        assert_eq!(score_content_hash("foobar"), "85944171f73967e8");
        assert_eq!(score_content_hash("<score-partwise/>"), "d374142b6cd125d2");
    }

    /// #385 AC1: importing the same file twice yields one list entry, and the
    /// second import hands back that entry (so the UI can open it).
    #[test]
    fn reimporting_identical_content_reuses_the_existing_entry() {
        let store = ScoreStore::in_memory().unwrap();
        let xml = "<?xml version=\"1.0\"?><score-partwise><!-- kit --></score-partwise>";

        let first = store
            .import(
                "Test Scale (C major)".to_string(),
                Some("AMC Test Kit".to_string()),
                "test-scale.musicxml".to_string(),
                xml.to_string(),
                0,
                4,
            )
            .unwrap();
        let second = store
            .import(
                "Test Scale (C major)".to_string(),
                Some("AMC Test Kit".to_string()),
                "test-scale.musicxml".to_string(),
                xml.to_string(),
                0,
                4,
            )
            .unwrap();

        assert_eq!(second.id, first.id, "re-import must reuse the entry");
        assert_eq!(second.music_xml, xml);
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1, "no duplicate row for identical content");
        assert_eq!(list[0].id, first.id);
    }

    /// #385 AC2: a genuinely different file with the same title is a
    /// different piece — it must still get its own entry.
    #[test]
    fn same_title_different_content_stays_two_entries() {
        let store = ScoreStore::in_memory().unwrap();

        let first = store
            .import(
                "Etude No. 1".to_string(),
                None,
                "etude.musicxml".to_string(),
                "<score-partwise><!-- in C --></score-partwise>".to_string(),
                0,
                8,
            )
            .unwrap();
        let second = store
            .import(
                "Etude No. 1".to_string(),
                None,
                "etude.musicxml".to_string(),
                "<score-partwise><!-- in Db --></score-partwise>".to_string(),
                0,
                8,
            )
            .unwrap();

        assert_ne!(second.id, first.id, "same title is not the same piece");
        assert_eq!(store.list().unwrap().len(), 2);
    }

    /// Importing another part of the same multi-part file is a distinct
    /// library entry: the stored MusicXML is identical, the part isn't.
    #[test]
    fn same_content_different_part_stays_two_entries() {
        let store = ScoreStore::in_memory().unwrap();
        let xml = "<score-partwise><!-- duet --></score-partwise>".to_string();

        let flute = store
            .import(
                "Duet".to_string(),
                None,
                "duet.musicxml".to_string(),
                xml.clone(),
                0,
                12,
            )
            .unwrap();
        let oboe = store
            .import(
                "Duet".to_string(),
                None,
                "duet.musicxml".to_string(),
                xml,
                1,
                12,
            )
            .unwrap();

        assert_ne!(oboe.id, flute.id, "part 1 is not part 0");
        assert_eq!(store.list().unwrap().len(), 2);
    }

    /// A library created before the `content_hash` column existed must dedup
    /// against its legacy rows: `migrate()` backfills their hashes, so
    /// re-importing an old score lands on the old entry, not a duplicate.
    #[test]
    fn migration_backfills_hashes_so_legacy_rows_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.db");
        let xml = "<score-partwise><!-- legacy --></score-partwise>";
        let legacy_id = ScoreId::new();

        // Build the pre-#385 scores table by hand: no content_hash column.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE scores (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    composer TEXT,
                    source_filename TEXT NOT NULL,
                    added_at TEXT NOT NULL,
                    last_practiced_at TEXT,
                    part_index INTEGER NOT NULL DEFAULT 0,
                    duration_measures INTEGER NOT NULL DEFAULT 0,
                    music_xml TEXT NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO scores (id, title, composer, source_filename, added_at, part_index, duration_measures, music_xml) \
                 VALUES (?1, 'Legacy', NULL, 'legacy.musicxml', '2026-01-01T00:00:00+00:00', 0, 4, ?2)",
                params![legacy_id.as_str(), xml],
            )
            .unwrap();
        }

        let store = ScoreStore::open(&path).unwrap();
        let entry = store
            .import(
                "Legacy".to_string(),
                None,
                "legacy.musicxml".to_string(),
                xml.to_string(),
                0,
                4,
            )
            .unwrap();

        assert_eq!(
            entry.id, legacy_id,
            "re-import must land on the backfilled legacy row"
        );
        assert_eq!(store.list().unwrap().len(), 1);
    }

    /// Review survivor 1: the blob-equality clause is the collision guard —
    /// a row whose content_hash matches but whose MusicXML differs must NOT
    /// be reused. No real FNV collision needed: forge one with raw SQL.
    #[test]
    fn hash_collision_with_different_blob_creates_a_new_entry() {
        let store = ScoreStore::in_memory().unwrap();
        let new_xml = "<score-partwise><!-- the real import --></score-partwise>";
        // Seed a row that LIES about its hash: it claims new_xml's hash but
        // stores different content — exactly what a hash collision looks
        // like to the dedup SELECT.
        store
            .conn
            .execute(
                "INSERT INTO scores (id, title, composer, source_filename, added_at, part_index, duration_measures, music_xml, content_hash)                  VALUES (?1, 'Impostor', NULL, 'other.musicxml', '2026-01-01T00:00:00+00:00', 0, 4, '<score-partwise><!-- different --></score-partwise>', ?2)",
                params![ScoreId::new().as_str(), score_content_hash(new_xml)],
            )
            .unwrap();

        let entry = store
            .import(
                "Real".to_string(),
                None,
                "real.musicxml".to_string(),
                new_xml.to_string(),
                0,
                4,
            )
            .unwrap();

        assert_eq!(
            store.list().unwrap().len(),
            2,
            "a colliding hash with different content must not dedup"
        );
        assert_eq!(entry.title, "Real");
    }

    /// Review survivor 2: with pre-#385 duplicates in the library, a
    /// re-import must land on the NEWEST duplicate (the one whose practice
    /// history the user has been building since).
    #[test]
    fn dedup_prefers_the_newest_of_preexisting_duplicates() {
        let store = ScoreStore::in_memory().unwrap();
        let xml = "<score-partwise><!-- dup --></score-partwise>";
        let (old_id, new_id) = (ScoreId::new(), ScoreId::new());
        for (id, added) in [(&old_id, "2026-01-01"), (&new_id, "2026-06-01")] {
            store
                .conn
                .execute(
                    "INSERT INTO scores (id, title, composer, source_filename, added_at, part_index, duration_measures, music_xml, content_hash)                      VALUES (?1, 'Dup', NULL, 'dup.musicxml', ?2, 0, 4, ?3, ?4)",
                    params![id.as_str(), format!("{added}T00:00:00+00:00"), xml, score_content_hash(xml)],
                )
                .unwrap();
        }

        let entry = store
            .import(
                "Dup".to_string(),
                None,
                "dup.musicxml".to_string(),
                xml.to_string(),
                0,
                4,
            )
            .unwrap();

        assert_eq!(
            entry.id, new_id,
            "re-import must reuse the NEWEST duplicate, not the oldest"
        );
        assert_eq!(store.list().unwrap().len(), 2, "no third entry");
    }

    // -----------------------------------------------------------------------
    // #449 T1: practice_events, integrity columns, spec_hash
    // -----------------------------------------------------------------------

    /// A minimal phrase for integrity fixtures — only the fields
    /// `session_integrity` and `save_phrases` read are meaningful.
    fn t1_phrase(idx: usize, start: f64, end: f64, notes: usize) -> PhraseSummary {
        use crate::phrase::{DynamicsStats, PitchStats};
        PhraseSummary {
            phrase_index: idx,
            start_time: start,
            end_time: end,
            duration_secs: end - start,
            note_count: notes,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 435.0,
                max_hz: 445.0,
                range_cents: 40.0,
                pitches: vec![440.0; notes.max(1)],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.6,
                min_amplitude: 0.4,
                max_amplitude: 0.8,
                dynamic_range: 0.4,
            },
            stability: 0.9,
            score_position: None,
            tone: None,
            key: None,
            onsets_secs: Vec::new(),
            score_span: None,
            verdicts: None,
            score_card: None,
        }
    }

    /// A legacy (pre-T1, post-v1) database: sessions with the v1 columns but
    /// none of the integrity columns, exercise_log without spec_hash, no
    /// practice_events table, user_version pinned at 1.
    fn legacy_v1_store() -> SessionStore {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY, instrument TEXT NOT NULL,
                 started_at TEXT NOT NULL, ended_at TEXT NOT NULL,
                 duration_secs REAL NOT NULL, phrase_count INTEGER NOT NULL,
                 recap_json TEXT NOT NULL, score_id TEXT,
                 app_version TEXT, practice_mode TEXT);
             CREATE TABLE exercise_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 logged_at TEXT NOT NULL, source TEXT NOT NULL,
                 label TEXT NOT NULL, spec_json TEXT NOT NULL,
                 seed INTEGER NOT NULL, difficulty INTEGER NOT NULL,
                 tonic INTEGER NOT NULL, accuracy REAL);
             INSERT INTO sessions VALUES
                 ('s-old','Trumpet','2026-01-01T00:00:00+00:00',
                  '2026-01-01T00:30:00+00:00',1800.0,3,'{}',NULL,NULL,NULL);
             INSERT INTO exercise_log
                 (logged_at, source, label, spec_json, seed, difficulty, tonic, accuracy)
                 VALUES ('t0','opener','L','{\"cell\":[0,4,7]}',1,2,5,NULL);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        SessionStore { conn }
    }

    /// AC1: a legacy DB gains the whole T1 footprint exactly once — the
    /// practice_events table, the three integrity columns, spec_hash — and
    /// lands on user_version 2; a second migrate is a no-op. Catches a
    /// migration that forgets a column, re-runs the ALTERs, or never stamps
    /// the version (which would re-backfill on every launch).
    #[test]
    fn migration_v2_adds_telemetry_schema_to_legacy_db_once() {
        let store = legacy_v1_store();
        store.migrate().expect("v2 migration on a legacy DB");

        let cols: Vec<String> = store
            .conn
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for c in ["played_secs", "note_count", "silence_ratio"] {
            assert!(cols.iter().any(|n| n == c), "sessions must gain {c}");
        }
        let n: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM practice_events", [], |r| r.get(0))
            .expect("practice_events table must exist");
        assert_eq!(n, 0);
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        // Legacy integrity columns stay NULL — honest absence, never zeros.
        let (played, ratio): (Option<f64>, Option<f64>) = store
            .conn
            .query_row(
                "SELECT played_secs, silence_ratio FROM sessions WHERE id='s-old'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((played, ratio), (None, None));
        store.migrate().expect("second migrate is a no-op");
    }

    /// AC2: legacy exercise_log rows are backfilled with the FNV of their
    /// spec_json, and new rows get the hash at insert — so old and new
    /// attempts of one cell group under ONE key.
    #[test]
    fn migration_backfills_spec_hash_and_new_rows_match() {
        let store = legacy_v1_store();
        store.migrate().unwrap();

        let old_hash: Option<String> = store
            .conn
            .query_row("SELECT spec_hash FROM exercise_log WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            old_hash.as_deref(),
            Some(exercise_spec_hash("{\"cell\":[0,4,7]}").as_str()),
            "backfill must hash the stored spec_json bytes as-is"
        );

        store
            .log_exercise(&ExerciseLogEntry {
                source: "opener".to_owned(),
                label: "L".to_owned(),
                spec_json: "{\"cell\":[0,4,7]}".to_owned(),
                seed: 2,
                difficulty: 2,
                tonic: 9,
                accuracy: None,
            })
            .unwrap();
        let new_hash: Option<String> = store
            .conn
            .query_row("SELECT spec_hash FROM exercise_log WHERE id=2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            new_hash, old_hash,
            "a fresh log of the same spec must land in the same group as the backfilled row"
        );
    }

    /// F4 (retry-farming): N re-grades of one (spec, tonic) collapse to one
    /// spec_hash group of size N — the `max_retries` surface needs exactly
    /// this GROUP BY to work without parsing spec_json per query.
    #[test]
    fn retry_farming_groups_by_spec_hash() {
        let store = SessionStore::in_memory().unwrap();
        let entry = |spec: &str, tonic: u8| ExerciseLogEntry {
            source: "explore".to_owned(),
            label: "L".to_owned(),
            spec_json: spec.to_owned(),
            seed: 1,
            difficulty: 2,
            tonic,
            accuracy: Some(0.95),
        };
        for _ in 0..5 {
            store.log_exercise(&entry("{\"cell\":[0,3,7]}", 0)).unwrap();
        }
        store.log_exercise(&entry("{\"cell\":[0,4,7]}", 0)).unwrap();

        let max_retries: i64 = store
            .conn
            .query_row(
                "SELECT MAX(cnt) FROM (SELECT COUNT(*) AS cnt FROM exercise_log \
                 GROUP BY spec_hash, tonic)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            max_retries, 5,
            "the farmed exercise must dominate the GROUP BY"
        );
        let groups: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT spec_hash) FROM exercise_log",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(groups, 2, "distinct material must stay distinct");
    }

    /// F5 (key-camping): the same cell logged across tonics shares ONE
    /// spec_hash (spec_json carries no tonic), so the 12-key coverage matrix
    /// is `GROUP BY spec_hash, tonic` — one hot column exposes the camping.
    #[test]
    fn key_camping_shares_spec_hash_across_tonics() {
        let store = SessionStore::in_memory().unwrap();
        let entry = |tonic: u8| ExerciseLogEntry {
            source: "explore".to_owned(),
            label: "L".to_owned(),
            spec_json: "{\"cell\":[0,3,7]}".to_owned(),
            seed: 1,
            difficulty: 2,
            tonic,
            accuracy: Some(0.9),
        };
        // Camped: ten attempts in one comfortable key, one stray attempt in
        // another.
        for _ in 0..10 {
            store.log_exercise(&entry(0)).unwrap();
        }
        store.log_exercise(&entry(7)).unwrap();

        let hashes: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT spec_hash) FROM exercise_log",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hashes, 1, "one cell = one material row, across every key");
        let per_tonic: Vec<(i64, i64)> = store
            .conn
            .prepare(
                "SELECT tonic, COUNT(*) FROM exercise_log \
                 GROUP BY spec_hash, tonic ORDER BY COUNT(*) DESC",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            per_tonic,
            vec![(0, 10), (7, 1)],
            "the camped tonic must stand out as the one hot column"
        );
    }

    /// F1 (walk-away): zero phrases over a 30-minute wall reads as zero
    /// played time, zero notes, total silence — and the flags stay
    /// CONSISTENT with the #445-6b narration gates: zero phrases routes to
    /// the empty-state path (by `is_thin_session`'s documented contract,
    /// zero phrases is NOT "thin"), while a sparse near-walk-away session
    /// that DOES trip the thin gate also reads low on the same persisted
    /// played clock — the aggregates and the gates never contradict.
    #[test]
    fn session_integrity_walk_away_is_all_silence() {
        let recap_input = |phrases: Vec<PhraseSummary>| crate::session::RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 1800.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases,
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        // The pure walk-away: all silence on the persisted clock…
        let integrity = session_integrity(&[], 1800.0);
        assert_eq!(integrity.played_secs, 0.0);
        assert_eq!(integrity.note_count, 0);
        assert_eq!(integrity.silence_ratio, 1.0);
        // …and the empty-state narration path, NOT the thin one (the
        // documented zero-phrase rule — the founder's voice-reference copy).
        assert!(
            !crate::coaching::is_thin_session(&recap_input(Vec::new())),
            "zero phrases is the empty-state path, never 'thin'"
        );

        // The near-walk-away (one 10 s phrase in 30 min): thin to the
        // narration gate, and the SAME story on the persisted aggregates.
        let sparse = t1_phrase(0, 100.0, 110.0, 4);
        assert!(
            crate::coaching::is_thin_session(&recap_input(vec![sparse.clone()])),
            "a sparse session must trip the thin gate"
        );
        let sparse_integrity = session_integrity(&[sparse], 1800.0);
        assert!(
            sparse_integrity.played_secs < crate::coaching::THIN_SESSION_MIN_PLAYED_SECS,
            "the persisted played clock agrees with the thin threshold"
        );
        assert!(sparse_integrity.silence_ratio > 0.99);
    }

    /// The documented edges: zero wall → 1.0 (vacuous silence, not NaN);
    /// played exceeding wall (clock slop) clamps to 0.0; a normal session
    /// computes the straight ratio; corrupt negative phrase spans can't eat
    /// real played time.
    #[test]
    fn session_integrity_clamps_and_zero_wall() {
        assert_eq!(session_integrity(&[], 0.0).silence_ratio, 1.0);
        assert_eq!(session_integrity(&[], -5.0).silence_ratio, 1.0);

        let over = session_integrity(&[t1_phrase(0, 0.0, 700.0, 100)], 600.0);
        assert_eq!(
            over.silence_ratio, 0.0,
            "played > wall clamps, never negative"
        );

        let normal = session_integrity(
            &[t1_phrase(0, 0.0, 30.0, 40), t1_phrase(1, 100.0, 130.0, 50)],
            600.0,
        );
        assert!((normal.played_secs - 60.0).abs() < 1e-9);
        assert_eq!(normal.note_count, 90);
        assert!((normal.silence_ratio - 0.9).abs() < 1e-9);

        let corrupt = session_integrity(&[t1_phrase(0, 50.0, 40.0, 5)], 100.0);
        assert_eq!(
            corrupt.played_secs, 0.0,
            "a negative span clamps to zero instead of subtracting"
        );
    }

    /// AC7: the close-time aggregates round-trip onto the session row and
    /// surface on the summary; a row saved without them reads None (honest
    /// absence), never fabricated zeros.
    #[test]
    fn record_session_integrity_round_trips() {
        let store = SessionStore::in_memory().unwrap();
        let (with_id, without_id) = (SessionId::new(), SessionId::new());
        let now = Utc::now();
        store
            .save(
                with_id,
                now,
                now + Duration::seconds(600),
                &recap_with("trumpet", 600.0, 2),
            )
            .unwrap();
        store
            .save(
                without_id,
                now + Duration::seconds(700),
                now + Duration::seconds(760),
                &recap_with("trumpet", 60.0, 1),
            )
            .unwrap();

        let integrity = session_integrity(
            &[t1_phrase(0, 0.0, 30.0, 40), t1_phrase(1, 100.0, 130.0, 50)],
            600.0,
        );
        store.record_session_integrity(with_id, &integrity).unwrap();

        let summaries = store.list_recent(10).unwrap();
        let with = summaries.iter().find(|s| s.id == with_id).unwrap();
        assert_eq!(with.played_secs, Some(60.0));
        assert_eq!(with.note_count, Some(90));
        assert_eq!(with.silence_ratio, Some(0.9));
        let without = summaries.iter().find(|s| s.id == without_id).unwrap();
        assert_eq!(
            (
                without.played_secs,
                without.note_count,
                without.silence_ratio
            ),
            (None, None, None),
            "a session closed before the aggregates ran must read as unknown"
        );
    }

    /// AC3 (store half): the journal appends and reads back in session-clock
    /// order regardless of insert order, scoped to its session.
    #[test]
    fn practice_events_roundtrip_in_clock_order() {
        let store = SessionStore::in_memory().unwrap();
        store
            .log_practice_event("s1", 120.0, "pocket_stop", "{\"bpm\":96.0}")
            .unwrap();
        store
            .log_practice_event("s1", 10.0, "pocket_start", "{\"bpm\":90.0}")
            .unwrap();
        store
            .log_practice_event("s2", 5.0, "band_start", "{}")
            .unwrap();

        let events = store.list_practice_events("s1").unwrap();
        assert_eq!(
            events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec!["pocket_start", "pocket_stop"],
            "one session's journal, ordered by at_secs"
        );
        assert_eq!(events[0].at_secs, 10.0);
        assert_eq!(events[0].params_json, "{\"bpm\":90.0}");
        assert_eq!(store.count_practice_events().unwrap(), 3);
    }

    /// F8 (tool-on-with-no-notes): pocket events range-join against phrases
    /// with near-zero note_count — the "metronome ran, nothing was played
    /// into it" span is one SQL query over the two local tables.
    #[test]
    fn pocket_on_span_with_no_notes_is_joinable() {
        let store = SessionStore::in_memory().unwrap();
        let id = SessionId::new();
        let now = Utc::now();
        store
            .save(
                id,
                now,
                now + Duration::seconds(300),
                &recap_with("trumpet", 300.0, 2),
            )
            .unwrap();
        // Two phrases inside the click's span: one silent-ish, one real.
        store
            .save_phrases(
                id,
                &[t1_phrase(0, 30.0, 60.0, 0), t1_phrase(1, 70.0, 100.0, 25)],
            )
            .unwrap();
        let sid = id.as_str();
        store
            .log_practice_event(&sid, 20.0, "pocket_start", "{\"bpm\":90.0}")
            .unwrap();
        store
            .log_practice_event(&sid, 110.0, "pocket_stop", "{\"bpm\":90.0}")
            .unwrap();

        // The integrity panel's F8 join: phrases with no voiced notes that
        // sit inside a pocket_start..pocket_stop window.
        let silent_in_span: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM session_phrases p \
                 JOIN practice_events a ON a.session_id = p.session_id AND a.kind = 'pocket_start' \
                 JOIN practice_events b ON b.session_id = p.session_id AND b.kind = 'pocket_stop' \
                 WHERE p.session_id = ?1 AND p.note_count = 0 \
                   AND p.start_secs >= a.at_secs AND p.end_secs <= b.at_secs",
                params![sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            silent_in_span, 1,
            "exactly the silent phrase inside the click span"
        );
    }

    /// AC3 (failure half): the raw store writer DOES surface an Err when the
    /// journal is unwritable — proving the command layer's `()`-returning
    /// wrapper has something real to swallow (if this stopped failing, the
    /// best-effort posture would be untested theater).
    #[test]
    fn log_practice_event_err_is_surfaced_to_the_swallowing_caller() {
        let store = SessionStore::in_memory().unwrap();
        store
            .conn
            .execute_batch("DROP TABLE practice_events;")
            .unwrap();
        let result = store.log_practice_event("s1", 1.0, "pocket_start", "{}");
        assert!(
            result.is_err(),
            "an unwritable journal must error at the store seam (the command \
             layer, not the store, is where best-effort lives)"
        );
    }
}
