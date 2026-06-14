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
/// [`SessionSummary`]. All three list_* methods query the same five
/// columns in the same order (`id, instrument, started_at,
/// duration_secs, phrase_count`), so the decode logic lives here once.
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
    score_id TEXT REFERENCES scores(id) ON DELETE SET NULL
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
    music_xml TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_scores_last_practiced ON scores(last_practiced_at DESC, added_at DESC);
CREATE TABLE IF NOT EXISTS taste_profile (
    user_id TEXT PRIMARY KEY,
    profile_json TEXT NOT NULL,
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
";

/// The `user_id` used for the single local taste profile before any cloud
/// account exists. Local-first: the profile is captured at onboarding with no
/// sign-in required; if the user later opts into sync, the row is projected up
/// keyed on their real `profiles.id`. Mirrors the local-then-synced model the
/// session data already uses.
pub const LOCAL_TASTE_PROFILE_USER_ID: &str = "local";

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
            self.add_column_if_missing("sessions", "score_id", "TEXT")?;
            self.add_column_if_missing("sessions", "app_version", "TEXT")?;
            self.add_column_if_missing("sessions", "practice_mode", "TEXT")?;
            self.conn.execute_batch("PRAGMA user_version = 1;")?;
        }
        Ok(())
    }

    /// Add `column` to `table` only if absent. SQLite's
    /// `ALTER TABLE ADD COLUMN` is not idempotent, so we check `table_info`
    /// first — keeping `migrate()` safe to run on fresh and already-migrated
    /// databases alike. `table`/`column`/`decl` are internal constants, never
    /// user input.
    fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        decl: &str,
    ) -> Result<(), StoreError> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|name| name == column);
        drop(stmt);
        if !exists {
            self.conn
                .execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))?;
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
            "SELECT id, instrument, started_at, duration_secs, phrase_count \
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
            "SELECT id, instrument, started_at, duration_secs, phrase_count \
             FROM sessions WHERE instrument = ?1 ORDER BY started_at DESC"
        } else {
            "SELECT id, instrument, started_at, duration_secs, phrase_count \
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
                "SELECT id, instrument, started_at, duration_secs, phrase_count \
                 FROM sessions WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at DESC",
                Some(s.to_rfc3339()),
                Some(e.to_rfc3339()),
            ),
            (Some(s), None) => (
                "SELECT id, instrument, started_at, duration_secs, phrase_count \
                 FROM sessions WHERE started_at >= ?1 ORDER BY started_at DESC",
                Some(s.to_rfc3339()),
                None,
            ),
            (None, Some(e)) => (
                "SELECT id, instrument, started_at, duration_secs, phrase_count \
                 FROM sessions WHERE started_at < ?1 ORDER BY started_at DESC",
                Some(e.to_rfc3339()),
                None,
            ),
            (None, None) => (
                "SELECT id, instrument, started_at, duration_secs, phrase_count \
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
        Ok(())
    }

    /// Import a score: parse it, validate it, persist it to the library.
    ///
    /// Returns the [`ScoreLibraryEntry`] that was stored.
    /// `music_xml` is the raw MusicXML string (parsed by the caller from file).
    /// `title`, `composer`, and other metadata may come from the parsed content
    /// or be inferred from the filename.
    pub fn import(
        &self,
        title: String,
        composer: Option<String>,
        source_filename: String,
        music_xml: String,
        part_index: usize,
        duration_measures: usize,
    ) -> Result<ScoreLibraryEntry, StoreError> {
        let id = ScoreId::new();
        let now = Utc::now();

        self.conn.execute(
            "INSERT INTO scores \
             (id, title, composer, source_filename, added_at, part_index, duration_measures, music_xml) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id.as_str(),
                title,
                composer,
                source_filename.clone(),
                now.to_rfc3339(),
                part_index as i64,
                duration_measures as i64,
                music_xml.clone(),
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

    fn recap_with(instrument: &str, duration: f64, phrase_count: usize) -> SessionRecap {
        SessionRecap {
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
            fingerprint: Some(MusicalFingerprint {
                tone: Some(tone::ToneDescriptor {
                    brightness: 0.6,
                    warmth: 0.5,
                    air_noise: 0.2,
                    core_clarity: 0.8,
                    vibrato_quality: 0.55,
                }),
                key: None,
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
        let xml = "<?xml version=\"1.0\"?><score-partwise/>".to_string();

        let id1 = store
            .import(
                "Score 1".to_string(),
                None,
                "s1.musicxml".to_string(),
                xml.clone(),
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
                xml.clone(),
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
}
