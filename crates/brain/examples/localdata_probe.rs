//! Pre-release probe (#292/#285/#289/#256/#258): run the weekend's REAL read
//! paths against a copy of the founder's actual practice database (June 13–18
//! sessions — the exact "upgraded old install" shape Monday's testers have).
//!
//!   cargo run -p brain --example localdata_probe -- /path/to/sessions.db
//!
//! Read-only intent: run it against a COPY. Exits non-zero on any panic-level
//! surprise; prints a finding per feature otherwise.

use brain::coach::{lift_cell_from_pitch_track, start_explore_cell, suggest_chips, LIFT_MAX_NOTES};
use brain::mirror::derive_sound_profile;
use brain::score::cellstaff::cell_staff_view;
use brain::store::SessionStore;
use brain::wheel::build_wheel;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: localdata_probe <sessions.db>");
    let store = SessionStore::open(std::path::Path::new(&path)).expect("open real DB copy");

    // ── 1. Old-schema store opens + new reads behave ────────────────────────
    let summaries = store.list_recent(50).expect("list_recent on legacy DB");
    println!("[store] {} real sessions load cleanly", summaries.len());

    let model = store
        .get_learner_model("local")
        .expect("learner model read on a DB that predates the table");
    println!(
        "[learner] legacy DB → model = {:?} (None expected: table absent pre-weekend)",
        model.as_ref().map(|m| m.version)
    );
    let model = model.unwrap_or_default();

    // ── 2. Wheel + mirror over REAL recaps/fingerprints ────────────────────
    let mut fingerprints = Vec::new();
    let mut with_fp = 0usize;
    for s in &summaries {
        if let Ok(recap) = store.load_recap(s.id) {
            if let Some(fp) = recap.fingerprint {
                with_fp += 1;
                fingerprints.push(fp);
            }
        }
    }
    fingerprints.reverse();
    println!(
        "[recaps] {} of {} carry fingerprints",
        with_fp,
        summaries.len()
    );
    let wheel = build_wheel(&model, &fingerprints);
    println!(
        "[wheel] owned {}/12 · intonation {:?} · tone {:?} (honest empties expected)",
        wheel.total_owned, wheel.intonation_trend, wheel.tone_trend
    );
    let taste = store
        .get_taste_profile("local")
        .expect("taste read")
        .unwrap_or_default();
    match derive_sound_profile(&fingerprints, &taste, 0) {
        Some(p) => println!(
            "[mirror] {} sessions → lean {:?} feel {:?} conf {:.2} comparison {:?}",
            p.sessions_counted,
            p.mode_lean,
            p.feel,
            p.confidence,
            p.comparison.map(|c| c.label)
        ),
        None => println!(
            "[mirror] below threshold ({} measured) → honest empty state",
            fingerprints.len()
        ),
    }

    // ── 3. #285 lift over 135 REAL played phrases (voice + trumpet) ────────
    let mut liftable = 0usize;
    let mut total = 0usize;
    let mut examples: Vec<String> = Vec::new();
    for s in &summaries {
        let phrases = store.load_phrases(s.id).expect("load real phrases");
        for p in phrases {
            total += 1;
            if let Some((cell, first)) = lift_cell_from_pitch_track(&p.pitch_stats.pitches, 3) {
                liftable += 1;
                assert!(cell.len() <= LIFT_MAX_NOTES, "cap violated on real data");
                assert!(
                    cell.iter().all(|&o| (-36..=36).contains(&o)),
                    "range violated"
                );
                if examples.len() < 3 {
                    examples.push(format!("root {} cell {:?}", first, cell));
                }
                // Row + render the REAL lick through the full pipeline.
                let (state, seq) = start_explore_cell(cell, first % 12, &model, 7);
                let chips = suggest_chips(&state, &model);
                assert!(!chips.is_empty() && chips.len() <= 3);
                let key = brain::coach::key_signature_for(state.tonic, "major");
                let staff = cell_staff_view(&seq, key);
                assert_eq!(staff.notes.len(), seq.target_midi.len());
                assert!(staff.notes.iter().all(|n| (-30..=40).contains(&n.step)));
            }
        }
    }
    println!(
        "[lift] {}/{} real phrases lift into cells; e.g. {:?}",
        liftable, total, examples
    );

    // ── 4. Lift-quality experiment: stricter min_run on the same phrases ───
    for run in [3usize, 5, 6, 8] {
        let mut count = 0usize;
        let mut ex = Vec::new();
        for s in &summaries {
            for p in store.load_phrases(s.id).expect("phrases") {
                if let Some((cell, _)) = lift_cell_from_pitch_track(&p.pitch_stats.pitches, run) {
                    count += 1;
                    if ex.len() < 2 {
                        ex.push(format!("{cell:?}"));
                    }
                }
            }
        }
        println!("[tune] min_run={run}: {count} liftable; e.g. {ex:?}");
    }

    println!("PROBE OK");
}
