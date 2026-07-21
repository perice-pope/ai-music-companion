//! Golden-session gate (autonomy loop): every PR must survive the exact
//! upgrade path a real tester hits Monday morning — a database written by an
//! OLD build (no learner_model, no exercise_log tables; recaps with and
//! without fingerprints; noisy real-world pitch tracks) opened by the NEW
//! code, with every weekend read-path exercised end to end.
//!
//! The fixture shapes are modeled on the founder's actual June practice
//! database (8 sessions, 135 phrases): voice tracks carry ±1-semitone
//! flicker and re-struck notes; one phrase is a clean melodic lick.

use brain::coach::{lift_cell_from_pitch_track, start_explore_cell, LIFT_MIN_ROOTS, LIFT_MIN_RUN};
use brain::insights::exercise_insights;
use brain::mirror::derive_sound_profile;
use brain::score::cellstaff::cell_staff_view;
use brain::store::{ExerciseLogEntry, SessionStore};
use brain::wheel::{build_wheel, Trend};

/// The pre-weekend schema, verbatim shape (no learner_model, no exercise_log).
const LEGACY_SCHEMA: &str = "
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    instrument TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    duration_secs REAL NOT NULL,
    phrase_count INTEGER NOT NULL,
    recap_json TEXT NOT NULL
, score_id TEXT, app_version TEXT, practice_mode TEXT);
CREATE TABLE session_phrases (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    phrase_index INTEGER NOT NULL,
    note_count INTEGER NOT NULL,
    start_secs REAL NOT NULL,
    end_secs REAL NOT NULL,
    phrase_json TEXT NOT NULL,
    PRIMARY KEY (session_id, phrase_index)
);
CREATE TABLE scores (
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
CREATE TABLE taste_profile (
    user_id TEXT PRIMARY KEY, profile_json TEXT NOT NULL, updated_at TEXT NOT NULL
);
";

fn midi_hz(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

/// A phrase JSON in the store's shape, with the given pitch track.
fn phrase_json(index: usize, pitches: &[f64]) -> String {
    serde_json::json!({
        "phrase_index": index,
        "start_time": index as f64 * 4.0,
        "end_time": index as f64 * 4.0 + 3.0,
        "duration_secs": 3.0,
        "note_count": pitches.len(),
        "onsets_secs": [0.0, 0.5, 1.0],
        "pitch_stats": {
            "mean_hz": 220.0, "min_hz": 100.0, "max_hz": 800.0,
            "range_cents": 1200.0,
            "pitches": pitches
        },
        "dynamics": {
            "mean_amplitude": 0.05, "min_amplitude": 0.02,
            "max_amplitude": 0.08, "dynamic_range": 0.06
        },
        "stability": 0.71,
        "score_position": null,
        "tone": null,
        "key": null
    })
    .to_string()
}

fn recap_json(with_fingerprint: bool) -> String {
    let mut recap = serde_json::json!({
        "overall_assessment": "legacy recap",
        "strengths": ["tone"], "areas_to_improve": ["timing"],
        "next_session_suggestions": ["play more"],
        "duration_secs": 60.0, "phrase_count": 3, "instrument": "Voice"
    });
    if with_fingerprint {
        recap["fingerprint"] = serde_json::json!({
            "key": { "tonic": 2, "mode": "dorian", "confidence": 0.8, "margin": 0.2 },
            "groove": { "tempo_bpm": 100.0, "swing_ratio": 1.4,
                         "timing_consistency": 0.8, "mean_ioi_secs": 0.3, "onset_count": 24 }
        });
    }
    recap.to_string()
}

/// Build the legacy DB exactly as an old build would have left it.
fn write_legacy_db(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(LEGACY_SCHEMA).unwrap();
    for i in 0..6 {
        let id = format!("00000000-0000-4000-8000-00000000000{i}");
        conn.execute(
            "INSERT INTO sessions (id, started_at, ended_at, duration_secs, phrase_count, instrument, recap_json)
             VALUES (?1, ?2, ?3, 60.0, 3, 'Voice', ?4)",
            rusqlite::params![
                id,
                format!("2026-06-1{i}T12:00:00+00:00"),
                format!("2026-06-1{i}T12:01:00+00:00"),
                recap_json(i < 4) // 4 with fingerprints, 2 legacy-bare
            ],
        )
        .unwrap();
        // Phrase 0: voice-like jitter (flicker runs too short to lift).
        let jitter: Vec<f64> = (0..40)
            .map(|k| midi_hz(60.0 + f64::from((k % 3 == 0) as u8)))
            .collect();
        // Phrase 1: a clean 5-note lick,each note held 6 samples.
        let lick: Vec<f64> = [62.0, 65.0, 64.0, 69.0, 62.0]
            .iter()
            .flat_map(|&m| std::iter::repeat_n(midi_hz(m), 6))
            .collect();
        for (idx, track) in [jitter, lick].into_iter().enumerate() {
            conn.execute(
                "INSERT INTO session_phrases (session_id, phrase_index, note_count, start_secs, end_secs, phrase_json)
                 VALUES (?1, ?2, 5, 0.0, 3.0, ?3)",
                rusqlite::params![id, idx as i64, phrase_json(idx, &track)],
            )
            .unwrap();
        }
    }
}

/// The gate: a legacy DB opens, migrates, and every new read/write path
/// behaves — honestly on missing data, correctly on real-shaped data.
#[test]
fn legacy_database_survives_the_whole_new_stack() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("legacy.db");
    write_legacy_db(&db);

    // Opening migrates in place (adds learner_model / exercise_log).
    let store = SessionStore::open(&db).expect("legacy DB must open and migrate");
    let sessions = store.list_recent(50).expect("legacy sessions list");
    assert_eq!(sessions.len(), 6);

    // Learner model: absent pre-upgrade → honest None, usable default.
    let model = store.get_learner_model("local").expect("read");
    assert!(
        model.is_none(),
        "no learner data existed before the upgrade"
    );
    let model = model.unwrap_or_default();

    // Wheel + mirror over the real recap mix (4 fingerprints, 2 bare).
    let mut fps = Vec::new();
    for s in &sessions {
        if let Ok(r) = store.load_recap(s.id) {
            if let Some(fp) = r.fingerprint {
                fps.push(fp);
            }
        }
    }
    fps.reverse();
    assert_eq!(fps.len(), 4, "exactly the fingerprinted recaps survive");
    let wheel = build_wheel(&model, &fps);
    assert_eq!(wheel.total_owned, 0, "no mastery yet — nothing may glow");
    assert_eq!(wheel.intonation_trend, Trend::Unknown, "no intonation data");
    let mirror = derive_sound_profile(&fps, &Default::default(), 0);
    assert!(
        mirror.is_none(),
        "4 measured sessions < threshold → dark mirror"
    );

    // The lift on real-shaped tracks: jitter refuses, the lick lifts and
    // rows through several keys onto a sane staff.
    let mut lifted = None;
    for s in &sessions {
        for p in store.load_phrases(s.id).expect("phrases") {
            if let Some(hit) = lift_cell_from_pitch_track(&p.pitch_stats.pitches, LIFT_MIN_RUN) {
                lifted = Some(hit);
            }
        }
    }
    let (cell, first) = lifted.expect("the clean lick must lift");
    assert_eq!(cell, vec![0, 3, 2, 7, 0]);
    let (state, seq) = start_explore_cell(
        cell,
        first % 12,
        &model,
        7,
        brain::coach::DirectionMode::Forward,
    );
    assert!(seq.root_order.len() >= LIFT_MIN_ROOTS);
    let staff = cell_staff_view(
        &seq,
        brain::coach::key_signature_for(state.tonic, "major"),
        "major",
    );
    assert_eq!(staff.notes.len(), seq.target_midi.len());

    // Exercise log: absent pre-upgrade → empty read, working append.
    assert!(store.list_exercise_log().expect("log reads").is_empty());
    store
        .log_exercise(&ExerciseLogEntry {
            source: "lesson".to_owned(),
            label: "post-upgrade".to_owned(),
            spec_json: "{}".to_owned(),
            seed: 1,
            difficulty: 0,
            tonic: 0,
            accuracy: Some(0.9),
        })
        .expect("append works on the migrated DB");
    let insights = exercise_insights(&store.list_exercise_log().unwrap());
    assert_eq!(insights.len(), 1);
}
