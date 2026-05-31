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
";

// ---------------------------------------------------------------------------
// SessionStore
// ---------------------------------------------------------------------------

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
            session_tone: None,
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
    fn session_tone_persists_and_legacy_recaps_load_as_none() {
        let store = SessionStore::in_memory().unwrap();

        // A recap carrying a session tone aggregate round-trips through the
        // recap_json column.
        let id = SessionId::new();
        let now = Utc::now();
        let recap = SessionRecap {
            session_tone: Some(tone::ToneDescriptor {
                brightness: 0.6,
                warmth: 0.5,
                air_noise: 0.2,
                core_clarity: 0.8,
                vibrato_quality: 0.55,
            }),
            ..recap_with("voice", 60.0, 9)
        };
        store.save(id, now, now, &recap).unwrap();
        let loaded = store.load(id).unwrap();
        assert_eq!(loaded.recap.session_tone, recap.session_tone);

        // A recap_json saved before `session_tone` existed (field absent) must
        // still deserialize, defaulting to None.
        let legacy: SessionRecap = serde_json::from_str(
            r#"{"overall_assessment":"ok","strengths":[],"areas_to_improve":[],
                "next_session_suggestions":[],"duration_secs":1.0,"phrase_count":1,
                "instrument":"flute"}"#,
        )
        .expect("legacy recap deserializes");
        assert!(legacy.session_tone.is_none());
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
            session_tone: None,
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
