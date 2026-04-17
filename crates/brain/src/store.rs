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
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::session::{SessionId, SessionRecap};

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
    recap_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at DESC);
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
        self.conn.execute(
            "INSERT OR REPLACE INTO sessions \
             (id, instrument, started_at, ended_at, duration_secs, phrase_count, recap_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id.as_str(),
                recap.instrument,
                started_at.to_rfc3339(),
                ended_at.to_rfc3339(),
                recap.duration_secs,
                phrase_count,
                recap_json,
            ],
        )?;
        Ok(())
    }

    /// Load the recap for a specific session id.
    ///
    /// Returns [`StoreError::NotFound`] if the id is unknown.
    pub fn load(&self, id: SessionId) -> Result<SessionRecap, StoreError> {
        let id_str = id.as_str();
        let row: Option<String> = self
            .conn
            .query_row(
                "SELECT recap_json FROM sessions WHERE id = ?1",
                params![id_str],
                |r| r.get::<_, String>(0),
            )
            .optional()?;

        match row {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Err(StoreError::NotFound(id_str)),
        }
    }

    /// Return up to `limit` session summaries, most recent first
    /// (ordered by `started_at DESC`).
    pub fn list_recent(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, instrument, started_at, duration_secs, phrase_count \
             FROM sessions ORDER BY started_at DESC LIMIT ?1",
        )?;
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![limit_i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;

        let mut summaries = Vec::new();
        for row in rows {
            let (id_str, instrument, started_at_str, duration_secs, phrase_count) = row?;
            let id: SessionId = id_str.parse().map_err(|e: uuid::Error| {
                StoreError::CorruptRow(format!("invalid session id {id_str}: {e}"))
            })?;
            let started_at = DateTime::parse_from_rfc3339(&started_at_str)
                .map_err(|e| {
                    StoreError::CorruptRow(format!(
                        "invalid RFC3339 started_at {started_at_str}: {e}"
                    ))
                })?
                .with_timezone(&Utc);
            let phrase_count = usize::try_from(phrase_count).unwrap_or(0);
            summaries.push(SessionSummary {
                id,
                instrument,
                started_at,
                duration_secs,
                phrase_count,
            });
        }
        Ok(summaries)
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

        // Per-field equality, not whole-object JSON string comparison,
        // so we catch any silent coercion (e.g. f64 rounding).
        assert_eq!(loaded.overall_assessment, recap.overall_assessment);
        assert_eq!(loaded.strengths, recap.strengths);
        assert_eq!(loaded.areas_to_improve, recap.areas_to_improve);
        assert_eq!(
            loaded.next_session_suggestions,
            recap.next_session_suggestions
        );
        assert_eq!(loaded.phrase_count, recap.phrase_count);
        assert_eq!(loaded.instrument, recap.instrument);
        assert!(
            (loaded.duration_secs - recap.duration_secs).abs() < 1e-9,
            "duration roundtrip mismatch: {} vs {}",
            loaded.duration_secs,
            recap.duration_secs
        );
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
        };
        let now = Utc::now();
        store
            .save(id, now, now + Duration::seconds(124), &recap)
            .unwrap();

        let loaded = store.load(id).unwrap();
        assert_eq!(
            loaded.strengths.len(),
            3,
            "strengths vector length must be preserved"
        );
        assert_eq!(loaded.strengths[0], "A");
        assert_eq!(loaded.strengths[1], "B");
        assert_eq!(loaded.strengths[2], "C");
        assert_eq!(loaded.areas_to_improve.len(), 2);
        assert_eq!(loaded.areas_to_improve[0], "X");
        assert_eq!(loaded.areas_to_improve[1], "Y");
        assert_eq!(loaded.next_session_suggestions.len(), 3);
        assert_eq!(loaded.next_session_suggestions[2], "Three");
        assert_eq!(loaded.phrase_count, 17);
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
            (loaded.duration_secs - 123.456).abs() < 1e-9,
            "duration_secs must preserve f64 precision, got {}",
            loaded.duration_secs
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
            loaded.instrument, "violin",
            "second save with same id must overwrite the first row"
        );
        assert!((loaded.duration_secs - 20.0).abs() < 1e-9);
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
}
