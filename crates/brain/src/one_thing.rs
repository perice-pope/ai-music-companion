//! #216 S1 — "Your one thing today": the pure daily-pick engine.
//!
//! Reads what already accumulates (stored recaps' intonation fingerprints,
//! the Learner Model's key mastery), names the single highest-leverage fix
//! with cited evidence, and deals a targeted cell-through-the-row exercise
//! for it — the founder's card: "*Today: your 4th runs flat — here's a
//! 5-min fix.*"
//!
//! Pure and deterministic: no I/O, no clock reads, no LLM. `None` means "no
//! weakness clears its evidence bar" — the caller falls back to the roulette
//! warmup. Silence over lies (the #453/#445-6b discipline): below any bar,
//! the card must not fire. Cell-first per
//! `docs/architecture/rv-methodology.md`: the deliverable is a cell × row
//! deal; key/degree evidence only *aims* it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use variations::{
    DirectionMode, RhythmSpec, ScaleModifier, ScalePattern, ScaleType, VariationSpec,
};

use crate::insights::{TREND_ACCURACY_BAR, TREND_MIN_ATTEMPTS, TREND_RECENT_DAYS};
use crate::learner::Mastery;

/// Evidence window: sessions older than this many days (exclusive; exactly
/// this age is still in) say nothing about *today's* one thing.
pub const EVIDENCE_WINDOW_DAYS: i64 = 21;
/// Newest sessions considered inside the window — a marathon history must
/// not drown the recent signal.
pub const MAX_SESSIONS: usize = 20;
/// A degree testifies only with this many observations behind it in a
/// session — two grazed notes are noise, not a tendency.
pub const DEGREE_MIN_COUNT: u32 = 5;
/// Cents bar for one session's testimony. Sits below the 15¢ in-tune
/// tolerance on purpose: a *consistent* 10¢ lean is a tendency even when
/// single notes pass.
pub const DEGREE_CENTS_BAR: f32 = 10.0;
/// Same-sign sessions required before a tendency is claimed.
pub const DEGREE_MIN_SESSIONS: usize = 3;
/// Degree-drill tempo: slow enough to hear the lean.
pub const DEGREE_TEMPO_BPM: f64 = 60.0;
/// Row passes in a degree deal — 3 × 12 shuffled roots = 36 figures.
pub const DEGREE_ROW_PASSES: usize = 3;
/// Key-drill tempo: an unhurried scale pass.
pub const KEY_TEMPO_BPM: f64 = 80.0;
/// Severity normalizer: a 25¢ mean lean saturates degree severity at 1.0.
pub const DEGREE_SEVERITY_FULL_CENTS: f32 = 25.0;
/// Frequency normalizer: 10 attempts saturate key frequency at 1.0.
pub const KEY_FREQUENCY_FULL_ATTEMPTS: f32 = 10.0;
/// Recency half-life (days) shared by both candidate kinds.
pub const RECENCY_HALF_LIFE_DAYS: f32 = 7.0;

/// The 12 chromatic roots start here (C4) — same base as the warmup roulette.
const ROOT_BASE: u8 = 60;

/// One session's evidence row, extracted by the caller from
/// `Store::list_by_instrument` + `load_recap` (the fingerprint rides the
/// recap).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEvidence {
    pub started_at: DateTime<Utc>,
    pub instrument: String,
    pub intonation: Option<theory::IntonationSummary>,
}

/// Which weakness the pick targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusKind {
    DegreeTendency,
    KeyTrend,
}

/// The daily pick: one weakness, its evidence, and the dealt fix.
#[derive(Debug, Clone, PartialEq)]
pub struct DailyPick {
    pub kind: FocusKind,
    /// The card line: "Your 4th runs flat (-12¢ across 4 sessions)".
    pub headline: String,
    /// Compact citation (#453 style): sessions/dates/raw values.
    pub evidence: String,
    /// Final rank score — surfaced so tests (and local diagnostics) can
    /// assert the ranking, never shown to the user.
    pub leverage: f32,
    /// The dealt fix: cell × row, ready for `variations::generate`. The row
    /// order is FINAL here: the pick shuffles `roots` with `day_seed` itself
    /// and emits `randomize_roots: false`, so the pick fully determines the
    /// deal.
    pub spec: VariationSpec,
}

/// Name the single highest-leverage fix from local history, or `None` when
/// nothing clears its evidence bar. Deterministic for fixed inputs;
/// `day_seed` only permutes the dealt row order (headline, evidence, and
/// leverage are seed-independent).
pub fn daily_pick(
    history: &[SessionEvidence],
    key_mastery: &BTreeMap<String, Mastery>,
    instrument: &str,
    fixed_pitch: bool,
    now: DateTime<Utc>,
    day_seed: u64,
) -> Option<DailyPick> {
    // Push order encodes the tie-break ladder (degree before key, low
    // semitone before high, BTreeMap key order): `best` is replaced only on
    // STRICTLY greater leverage, so on exact equality the earlier push wins.
    let mut candidates: Vec<Candidate> = Vec::new();
    if !fixed_pitch {
        // #389/#417-4: on piano the tendency is the instrument's tuning, not
        // the player's ear — never critique intonation on fixed pitch.
        candidates.extend(degree_candidates(history, instrument, now));
    }
    candidates.extend(key_candidates(key_mastery, now));

    let mut best: Option<Candidate> = None;
    for cand in candidates {
        match &best {
            Some(b) if cand.leverage() <= b.leverage() => {}
            _ => best = Some(cand),
        }
    }
    best.map(|c| realize(c, day_seed))
}

enum Candidate {
    Degree(DegreeCandidate),
    Key(KeyCandidate),
}

impl Candidate {
    fn leverage(&self) -> f32 {
        match self {
            Candidate::Degree(d) => d.leverage,
            Candidate::Key(k) => k.leverage,
        }
    }
}

struct DegreeCandidate {
    semitones: u8,
    /// Signed mean of the testifying sessions' mean cents.
    mean_cents: f32,
    testifying: usize,
    considered: usize,
    newest: DateTime<Utc>,
    leverage: f32,
}

struct KeyCandidate {
    /// Raw mastery key, e.g. `"3:dorian"`.
    key: String,
    /// Signature-spelled display, e.g. `"Eb dorian"`.
    display: String,
    tonic: u8,
    scale: ScaleType,
    attempts: u32,
    accuracy_ewma: f32,
    age_days: i64,
    leverage: f32,
}

/// Days since `then`, floored at zero — clock skew (a future session) reads
/// as age 0, never a negative-duration panic.
fn age_days(now: DateTime<Utc>, then: DateTime<Utc>) -> f32 {
    (now - then).num_seconds().max(0) as f32 / 86_400.0
}

fn recency(days: f32) -> f32 {
    0.5_f32.powf(days / RECENCY_HALF_LIFE_DAYS)
}

fn degree_candidates(
    history: &[SessionEvidence],
    instrument: &str,
    now: DateTime<Utc>,
) -> Vec<Candidate> {
    // Rows with no intonation are dropped BEFORE the newest-N cap so they
    // truly contribute nothing — a silent session must not displace evidence.
    let mut rows: Vec<&SessionEvidence> = history
        .iter()
        .filter(|r| {
            r.instrument == instrument
                && r.intonation.is_some()
                && (now - r.started_at).num_seconds() <= EVIDENCE_WINDOW_DAYS * 86_400
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    rows.truncate(MAX_SESSIONS);
    let considered = rows.len();

    let mut out = Vec::new();
    for semitones in 0u8..12 {
        // One session testifies for (degree, sign) when the lean is both
        // well-observed and past the cents bar.
        let mut sharp: Vec<(f32, DateTime<Utc>)> = Vec::new();
        let mut flat: Vec<(f32, DateTime<Utc>)> = Vec::new();
        for row in &rows {
            let summary = row.intonation.as_ref().expect("filtered to Some above");
            let Some(t) = summary
                .tendencies
                .iter()
                .find(|t| t.semitones_from_tonic == semitones)
            else {
                continue;
            };
            if !t.mean_cents.is_finite()
                || t.count < DEGREE_MIN_COUNT
                || t.mean_cents.abs() < DEGREE_CENTS_BAR
            {
                continue;
            }
            if t.mean_cents > 0.0 {
                sharp.push((t.mean_cents, row.started_at));
            } else {
                flat.push((t.mean_cents, row.started_at));
            }
        }
        // A stable tendency leans one way only — any opposite-sign testimony
        // disqualifies the degree entirely; say nothing.
        let testimony = match (sharp.len(), flat.len()) {
            (s, 0) if s >= DEGREE_MIN_SESSIONS => sharp,
            (0, f) if f >= DEGREE_MIN_SESSIONS => flat,
            _ => continue,
        };
        let testifying = testimony.len();
        let mean_cents = testimony.iter().map(|(c, _)| c).sum::<f32>() / testifying as f32;
        let mean_abs = testimony.iter().map(|(c, _)| c.abs()).sum::<f32>() / testifying as f32;
        let newest = testimony
            .iter()
            .map(|(_, at)| *at)
            .max()
            .expect("testimony is non-empty");
        let frequency = testifying as f32 / considered as f32;
        let severity = (mean_abs / DEGREE_SEVERITY_FULL_CENTS).min(1.0);
        out.push(Candidate::Degree(DegreeCandidate {
            semitones,
            mean_cents,
            testifying,
            considered,
            newest,
            leverage: frequency * severity * recency(age_days(now, newest)),
        }));
    }
    out
}

fn key_candidates(key_mastery: &BTreeMap<String, Mastery>, now: DateTime<Utc>) -> Vec<Candidate> {
    let mut out = Vec::new();
    // BTreeMap iteration = sorted by mastery key = deterministic order.
    for (key, m) in key_mastery {
        // The #453 trend bars, verbatim — this candidate must never claim a
        // struggle the recap's own suggestion line wouldn't.
        if !m.accuracy_ewma.is_finite()
            || m.attempts < TREND_MIN_ATTEMPTS
            || m.accuracy_ewma >= TREND_ACCURACY_BAR
        {
            continue;
        }
        let age_secs = now.timestamp().saturating_sub(m.last_epoch_secs);
        if age_secs > TREND_RECENT_DAYS * 86_400 {
            continue; // not a live trend
        }
        let Some((tonic_str, mode)) = key.split_once(':') else {
            continue;
        };
        let Ok(tonic) = tonic_str.parse::<u8>() else {
            continue;
        };
        if tonic >= 12 || mode.is_empty() {
            continue;
        }
        // Can't deal what can't be named: a mode outside the generator's
        // scale space yields no exercise, so it yields no claim either.
        let Some(scale) = crate::coach::scale_for_mode_label(mode) else {
            continue;
        };
        let Some(display) = crate::insights::mastery_key_display(key) else {
            continue;
        };
        let frequency = (m.attempts as f32 / KEY_FREQUENCY_FULL_ATTEMPTS).min(1.0);
        // Clamp: a corrupt negative-but-finite EWMA must not outrank
        // everything.
        let severity =
            ((TREND_ACCURACY_BAR - m.accuracy_ewma) / TREND_ACCURACY_BAR).clamp(0.0, 1.0);
        let age_days = (age_secs.max(0) as f32) / 86_400.0;
        out.push(Candidate::Key(KeyCandidate {
            key: key.clone(),
            display,
            tonic,
            scale,
            attempts: m.attempts,
            accuracy_ewma: m.accuracy_ewma,
            age_days: (age_secs.max(0)) / 86_400,
            leverage: frequency * severity * recency(age_days),
        }));
    }
    out
}

fn realize(candidate: Candidate, day_seed: u64) -> DailyPick {
    match candidate {
        Candidate::Degree(d) => {
            let name = crate::coaching::degree_name(d.semitones);
            let dir = if d.mean_cents >= 0.0 { "sharp" } else { "flat" };
            DailyPick {
                kind: FocusKind::DegreeTendency,
                headline: format!(
                    "Your {name} runs {dir} ({:+.0}¢ across {} sessions)",
                    d.mean_cents, d.testifying,
                ),
                evidence: format!(
                    "degree {}: {} of {} sessions {dir} by ≥{:.0}¢ (mean {:+.0}¢), newest {}",
                    d.semitones,
                    d.testifying,
                    d.considered,
                    DEGREE_CENTS_BAR,
                    d.mean_cents,
                    d.newest.format("%Y-%m-%d"),
                ),
                leverage: d.leverage,
                spec: degree_spec(d.semitones, day_seed),
            }
        }
        Candidate::Key(k) => DailyPick {
            kind: FocusKind::KeyTrend,
            headline: format!(
                "{} keeps slipping ({}% over {} attempts)",
                k.display,
                (k.accuracy_ewma * 100.0).round() as i32,
                k.attempts,
            ),
            evidence: format!(
                "key_mastery {}: {} attempts, accuracy EWMA {:.2}, last attempt {}d ago",
                k.key, k.attempts, k.accuracy_ewma, k.age_days,
            ),
            leverage: k.leverage,
            spec: key_spec(k.tonic, k.scale, day_seed),
        },
    }
}

/// Approach, sit on the problem tone twice, resolve — over 3 independently
/// shuffled passes of the 12 chromatic roots. Degree 0 (a leaning tonic)
/// deliberately degenerates to the repeated-note cell: a long-tone sit on
/// the tonic, which is exactly the classical fix.
fn degree_spec(semitones: u8, day_seed: u64) -> VariationSpec {
    let s = semitones as i8;
    let mut roots = Vec::with_capacity(12 * DEGREE_ROW_PASSES);
    // Chained scramble steps (the warmup-roulette pattern) give each pass an
    // independent permutation from the one day seed.
    let mut stream = day_seed;
    for _ in 0..DEGREE_ROW_PASSES {
        stream = splitmix64(stream);
        let mut pcs: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        shuffle(&mut pcs, stream);
        roots.extend(pcs.iter().map(|pc| ROOT_BASE + pc));
    }
    VariationSpec {
        roots,
        cell: Some(vec![0, s, s, 0]),
        degrees: None,
        progression: None,
        scale: None,
        chord: None,
        interval: None,
        enclosure: None,
        direction: DirectionMode::Forward,
        rhythm: RhythmSpec {
            notes_per_beat: 1,
            tempo_bpm: DEGREE_TEMPO_BPM,
            ..RhythmSpec::default()
        },
        randomize_roots: false,
    }
}

/// The weak key's own scale through all 12 roots: weak tonic pinned first
/// (the RV first-root-fixed rule, done here), the remaining eleven
/// seed-shuffled. The weak key gets its reps *and* its eleven siblings —
/// difficulty is row exposure, not "harder keys".
fn key_spec(tonic: u8, scale: ScaleType, day_seed: u64) -> VariationSpec {
    let mut rest: Vec<u8> = (0u8..12).filter(|pc| *pc != tonic).collect();
    shuffle(&mut rest, splitmix64(day_seed));
    let mut roots = Vec::with_capacity(12);
    roots.push(ROOT_BASE + tonic);
    roots.extend(rest.iter().map(|pc| ROOT_BASE + pc));
    VariationSpec {
        roots,
        cell: None,
        degrees: None,
        progression: None,
        scale: Some(ScaleModifier {
            scale,
            pattern: ScalePattern::UpDown,
        }),
        chord: None,
        interval: None,
        enclosure: None,
        direction: DirectionMode::Forward,
        rhythm: RhythmSpec {
            notes_per_beat: 1,
            tempo_bpm: KEY_TEMPO_BPM,
            ..RhythmSpec::default()
        },
        randomize_roots: false,
    }
}

/// Same scramble `variations` seeds everything with; private there, and
/// five lines is cheaper than widening that crate's API for a mixer.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Seeded Fisher–Yates over a splitmix64 stream — deterministic, no RNG
/// state carried.
fn shuffle(slice: &mut [u8], mut state: u64) {
    for i in (1..slice.len()).rev() {
        state = splitmix64(state);
        let j = (state % (i as u64 + 1)) as usize;
        slice.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use theory::{DegreeTendency, IntonationSummary};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap()
    }

    fn summary(tendencies: Vec<(u8, f32, u32)>) -> IntonationSummary {
        IntonationSummary {
            note_count: 40,
            mean_cents: 0.0,
            mean_abs_cents: 8.0,
            in_tune_ratio: 0.8,
            tendencies: tendencies
                .into_iter()
                .map(|(semitones_from_tonic, mean_cents, count)| DegreeTendency {
                    semitones_from_tonic,
                    mean_cents,
                    count,
                })
                .collect(),
        }
    }

    fn sess(days_ago: i64, instrument: &str, tendencies: Vec<(u8, f32, u32)>) -> SessionEvidence {
        SessionEvidence {
            started_at: now() - chrono::Duration::days(days_ago),
            instrument: instrument.to_owned(),
            intonation: Some(summary(tendencies)),
        }
    }

    fn mastery(attempts: u32, accuracy_ewma: f32, age_days: i64) -> Mastery {
        Mastery {
            attempts,
            accuracy_ewma,
            owned: false,
            last_epoch_secs: now().timestamp() - age_days * 86_400,
            extra: serde_json::Map::new(),
        }
    }

    /// AC1 fixture: three trumpet sessions, each a well-observed flat 4th.
    fn flat_fourth_history() -> Vec<SessionEvidence> {
        vec![
            sess(0, "Trumpet", vec![(5, -12.0, 6)]),
            sess(3, "Trumpet", vec![(5, -12.0, 7)]),
            sess(6, "Trumpet", vec![(5, -12.0, 5)]),
        ]
    }

    fn assert_chromatic_permutation(roots: &[u8]) {
        let mut sorted: Vec<u8> = roots.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (60u8..72).collect::<Vec<u8>>(),
            "each pass must be a permutation of the 12 chromatic roots"
        );
    }

    #[test]
    fn flat_fourth_across_sessions_fires_degree_pick() {
        let pick = daily_pick(
            &flat_fourth_history(),
            &BTreeMap::new(),
            "Trumpet",
            false,
            now(),
            42,
        )
        .expect("three qualifying sessions must fire");
        assert_eq!(pick.kind, FocusKind::DegreeTendency);
        assert_eq!(pick.headline, "Your 4th runs flat (-12¢ across 3 sessions)");
        assert_eq!(
            pick.evidence,
            "degree 5: 3 of 3 sessions flat by ≥10¢ (mean -12¢), newest 2026-09-04"
        );
        assert_eq!(pick.spec.cell, Some(vec![0, 5, 5, 0]));
        assert_eq!(pick.spec.roots.len(), 36, "3 row passes of 12 roots");
        for pass in pick.spec.roots.chunks(12) {
            assert_chromatic_permutation(pass);
        }
        // "Independently shuffled passes" — three copies of one permutation
        // would defeat the muscle-memory scramble.
        let passes: Vec<&[u8]> = pick.spec.roots.chunks(12).collect();
        assert!(
            passes[0] != passes[1] || passes[1] != passes[2],
            "the three passes must not all share one permutation"
        );
        assert_eq!(pick.spec.rhythm.tempo_bpm, DEGREE_TEMPO_BPM);
        assert_eq!(pick.spec.rhythm.notes_per_beat, 1);
        assert_eq!(pick.spec.direction, DirectionMode::Forward);
        assert!(!pick.spec.randomize_roots, "the pick owns the shuffle");
        assert!(pick.spec.scale.is_none() && pick.spec.chord.is_none());
        // Leverage is the exact formula: 3/3 testifying × (12/25 severity)
        // × recency 1.0 (newest testifying session is today).
        assert!((pick.leverage - 0.48).abs() < 1e-6, "got {}", pick.leverage);
    }

    #[test]
    fn two_sessions_stay_silent() {
        let history = &flat_fourth_history()[..2];
        assert_eq!(
            daily_pick(history, &BTreeMap::new(), "Trumpet", false, now(), 42),
            None,
            "below DEGREE_MIN_SESSIONS the card must not guess"
        );
    }

    #[test]
    fn direction_flip_disqualifies_degree() {
        let mut history = flat_fourth_history();
        history.push(sess(1, "Trumpet", vec![(5, 11.0, 6)]));
        assert_eq!(
            daily_pick(&history, &BTreeMap::new(), "Trumpet", false, now(), 42),
            None,
            "a degree that flips direction is not a stable tendency"
        );
    }

    #[test]
    fn weak_key_deals_its_mode_with_tonic_first() {
        let mut km = BTreeMap::new();
        km.insert("3:dorian".to_owned(), mastery(6, 0.4, 0));
        // Worse stats, but "bebop" maps to no generator scale — it must be
        // skipped before ranking, not deal an exercise it can't name.
        km.insert("0:bebop".to_owned(), mastery(10, 0.0, 0));
        let pick = daily_pick(&[], &km, "Trumpet", false, now(), 7)
            .expect("a qualifying key trend must fire");
        assert_eq!(pick.kind, FocusKind::KeyTrend);
        assert_eq!(pick.spec.roots[0], 63, "weak tonic pinned first");
        assert_chromatic_permutation(&pick.spec.roots);
        assert_eq!(
            pick.spec.scale,
            Some(ScaleModifier {
                scale: ScaleType::Dorian,
                pattern: ScalePattern::UpDown,
            })
        );
        assert_eq!(
            pick.headline,
            "Eb dorian keeps slipping (40% over 6 attempts)"
        );
        assert_eq!(
            pick.evidence,
            "key_mastery 3:dorian: 6 attempts, accuracy EWMA 0.40, last attempt 0d ago"
        );
        assert_eq!(pick.spec.rhythm.tempo_bpm, KEY_TEMPO_BPM);
        assert_eq!(pick.spec.rhythm.notes_per_beat, 1);
        assert!(!pick.spec.randomize_roots);
    }

    #[test]
    fn higher_leverage_wins_and_tie_prefers_degree() {
        // Degree leverage 0.5 (3/3 × 12.5/25 × 1.0) loses to key leverage
        // 1.0 (10 attempts, EWMA 0.0, fresh).
        let history = vec![
            sess(0, "Trumpet", vec![(5, -12.5, 6)]),
            sess(0, "Trumpet", vec![(5, -12.5, 6)]),
            sess(0, "Trumpet", vec![(5, -12.5, 6)]),
        ];
        let mut km = BTreeMap::new();
        km.insert("2:major".to_owned(), mastery(10, 0.0, 0));
        let pick = daily_pick(&history, &km, "Trumpet", false, now(), 7).unwrap();
        assert_eq!(pick.kind, FocusKind::KeyTrend, "higher leverage must win");

        // At exactly equal leverage (both 1.0) the degree wins — the more
        // specific coaching.
        let history = vec![
            sess(0, "Trumpet", vec![(5, -25.0, 6)]),
            sess(0, "Trumpet", vec![(5, -25.0, 6)]),
            sess(0, "Trumpet", vec![(5, -25.0, 6)]),
        ];
        let pick = daily_pick(&history, &km, "Trumpet", false, now(), 7).unwrap();
        assert_eq!(pick.leverage, 1.0);
        assert_eq!(
            pick.kind,
            FocusKind::DegreeTendency,
            "tie must prefer the degree"
        );
    }

    #[test]
    fn fixed_pitch_never_gets_intonation_critique() {
        let history = flat_fourth_history();
        assert_eq!(
            daily_pick(&history, &BTreeMap::new(), "Trumpet", true, now(), 42),
            None,
            "fixed pitch gates the degree source entirely (#389)"
        );
        // A qualifying key trend still fires — that one is the player's.
        let mut km = BTreeMap::new();
        km.insert("3:dorian".to_owned(), mastery(6, 0.4, 0));
        let pick = daily_pick(&history, &km, "Trumpet", true, now(), 42).unwrap();
        assert_eq!(pick.kind, FocusKind::KeyTrend);
    }

    #[test]
    fn stale_other_instrument_and_absent_rows_excluded() {
        // Baseline: the third testifying session sits at exactly 21 days —
        // the window edge is IN.
        let edge = vec![
            sess(0, "Trumpet", vec![(5, -12.0, 6)]),
            sess(3, "Trumpet", vec![(5, -12.0, 7)]),
            sess(21, "Trumpet", vec![(5, -12.0, 5)]),
        ];
        assert!(daily_pick(&edge, &BTreeMap::new(), "Trumpet", false, now(), 42).is_some());

        // Each exclusion removes one of the three needed rows → None.
        let mut other_instrument = flat_fourth_history();
        other_instrument[2].instrument = "Voice".to_owned();
        let mut too_old = flat_fourth_history();
        too_old[2].started_at = now() - chrono::Duration::days(22);
        let mut no_intonation = flat_fourth_history();
        no_intonation[2].intonation = None;
        for (label, history) in [
            ("other instrument", other_instrument),
            ("outside the 21-day window", too_old),
            ("intonation: None", no_intonation),
        ] {
            assert_eq!(
                daily_pick(&history, &BTreeMap::new(), "Trumpet", false, now(), 42),
                None,
                "a row excluded via {label} must contribute nothing"
            );
        }
    }

    #[test]
    fn same_seed_identical_different_seed_reorders_row() {
        let history = flat_fourth_history();
        let km = BTreeMap::new();
        let a = daily_pick(&history, &km, "Trumpet", false, now(), 7).unwrap();
        let b = daily_pick(&history, &km, "Trumpet", false, now(), 7).unwrap();
        assert_eq!(a, b, "same inputs and seed must be byte-identical");

        // Pinned seed pair known to permute differently: only the row order
        // moves; the claim is seed-independent.
        let c = daily_pick(&history, &km, "Trumpet", false, now(), 8).unwrap();
        assert_ne!(a.spec.roots, c.spec.roots);
        assert_eq!(a.kind, c.kind);
        assert_eq!(a.headline, c.headline);
        assert_eq!(a.evidence, c.evidence);
        assert_eq!(a.leverage, c.leverage);

        // The key deal keeps its tonic pinned under every seed.
        let mut weak = BTreeMap::new();
        weak.insert("3:dorian".to_owned(), mastery(6, 0.4, 0));
        let k1 = daily_pick(&[], &weak, "Trumpet", false, now(), 7).unwrap();
        let k2 = daily_pick(&[], &weak, "Trumpet", false, now(), 8).unwrap();
        assert_eq!(k1.spec.roots[0], 63);
        assert_eq!(k2.spec.roots[0], 63);
        assert_ne!(k1.spec.roots, k2.spec.roots);
    }

    #[test]
    fn recency_decays_with_evidence_age() {
        // Newest testifying session 7 days old = one half-life: leverage is
        // 3/3 frequency × (12/25) severity × 0.5 recency.
        let history = vec![
            sess(7, "Trumpet", vec![(5, -12.0, 6)]),
            sess(10, "Trumpet", vec![(5, -12.0, 6)]),
            sess(13, "Trumpet", vec![(5, -12.0, 6)]),
        ];
        let pick = daily_pick(&history, &BTreeMap::new(), "Trumpet", false, now(), 42).unwrap();
        assert!(
            (pick.leverage - 0.24).abs() < 1e-6,
            "week-old evidence must carry half the recency weight, got {}",
            pick.leverage
        );
    }

    #[test]
    fn future_session_counts_age_zero() {
        // Clock skew: a future-dated session reads as age 0, so recency caps
        // at 1.0 — leverage must equal AC1's 0.48, never exceed it.
        let history = vec![
            sess(-1, "Trumpet", vec![(5, -12.0, 6)]),
            sess(3, "Trumpet", vec![(5, -12.0, 6)]),
            sess(6, "Trumpet", vec![(5, -12.0, 6)]),
        ];
        let pick = daily_pick(&history, &BTreeMap::new(), "Trumpet", false, now(), 42).unwrap();
        assert!(
            (pick.leverage - 0.48).abs() < 1e-6,
            "a future session must not inflate recency past 1.0, got {}",
            pick.leverage
        );
    }

    #[test]
    fn corrupt_negative_ewma_severity_is_clamped() {
        // A corrupt negative-but-finite EWMA saturates severity at 1.0
        // instead of outranking everything (spec §4.2).
        let mut km = BTreeMap::new();
        km.insert("2:major".to_owned(), mastery(10, -5.0, 0));
        let pick = daily_pick(&[], &km, "Trumpet", false, now(), 42).unwrap();
        assert_eq!(
            pick.leverage, 1.0,
            "severity must clamp at 1.0 for a corrupt EWMA"
        );
    }

    #[test]
    fn newest_cap_applies_after_dropping_absent_rows() {
        // 20 newer sessions with intonation but no qualifying tendency push
        // the three testifiers past the MAX_SESSIONS cap → silence.
        let mut crowded: Vec<SessionEvidence> =
            (0..20).map(|i| sess(i % 3, "Trumpet", vec![])).collect();
        crowded.extend([
            sess(5, "Trumpet", vec![(5, -12.0, 6)]),
            sess(6, "Trumpet", vec![(5, -12.0, 6)]),
            sess(7, "Trumpet", vec![(5, -12.0, 6)]),
        ]);
        assert_eq!(
            daily_pick(&crowded, &BTreeMap::new(), "Trumpet", false, now(), 42),
            None,
            "capped-out testimony must not fire"
        );

        // But 20 newer rows WITHOUT intonation are dropped before the cap —
        // a silent session must not displace real evidence.
        let mut silent: Vec<SessionEvidence> = (0..20)
            .map(|i| SessionEvidence {
                started_at: now() - chrono::Duration::days(i % 3),
                instrument: "Trumpet".to_owned(),
                intonation: None,
            })
            .collect();
        silent.extend([
            sess(5, "Trumpet", vec![(5, -12.0, 6)]),
            sess(6, "Trumpet", vec![(5, -12.0, 6)]),
            sess(7, "Trumpet", vec![(5, -12.0, 6)]),
        ]);
        assert!(
            daily_pick(&silent, &BTreeMap::new(), "Trumpet", false, now(), 42).is_some(),
            "intonation-less rows must be dropped before the newest-20 cap"
        );
    }

    #[test]
    fn degree_tie_prefers_lower_semitone() {
        // Two degrees with identical stats → the lower semitone wins.
        let history = vec![
            sess(0, "Trumpet", vec![(5, -25.0, 6), (7, -25.0, 6)]),
            sess(0, "Trumpet", vec![(5, -25.0, 6), (7, -25.0, 6)]),
            sess(0, "Trumpet", vec![(5, -25.0, 6), (7, -25.0, 6)]),
        ];
        let pick = daily_pick(&history, &BTreeMap::new(), "Trumpet", false, now(), 42).unwrap();
        assert_eq!(pick.spec.cell, Some(vec![0, 5, 5, 0]));
        assert!(pick.headline.contains("4th"), "got: {}", pick.headline);
    }

    #[test]
    fn exact_cents_bar_testifies() {
        // A consistent lean of exactly 10¢ IS a tendency (the bar is ≥).
        let history = vec![
            sess(0, "Trumpet", vec![(5, -10.0, 5)]),
            sess(3, "Trumpet", vec![(5, -10.0, 5)]),
            sess(6, "Trumpet", vec![(5, -10.0, 5)]),
        ];
        assert!(
            daily_pick(&history, &BTreeMap::new(), "Trumpet", false, now(), 42).is_some(),
            "a 10¢ lean sits exactly on DEGREE_CENTS_BAR and must testify"
        );
    }

    #[test]
    fn production_mastery_labels_are_dealable() {
        // finish_lesson writes lowercased ScaleType labels ("harmonic
        // minor", …) into key_mastery — the ladder's own scales must round-
        // trip back into a deal, or weak keys at high difficulty go silent.
        let mut km = BTreeMap::new();
        km.insert("2:harmonic minor".to_owned(), mastery(8, 0.3, 0));
        let pick = daily_pick(&[], &km, "Trumpet", false, now(), 42)
            .expect("a ladder-written scale label must be dealable");
        assert_eq!(
            pick.spec.scale.map(|s| s.scale),
            Some(ScaleType::HarmonicMinor)
        );
    }

    #[test]
    fn empty_inputs_return_none() {
        assert_eq!(
            daily_pick(&[], &BTreeMap::new(), "Trumpet", false, now(), 42),
            None
        );
    }

    #[test]
    fn non_finite_values_are_skipped() {
        // NaN and infinite cents from legacy blobs never testify, never panic.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let history = vec![
                sess(0, "Trumpet", vec![(5, bad, 6)]),
                sess(1, "Trumpet", vec![(5, bad, 6)]),
                sess(2, "Trumpet", vec![(5, bad, 6)]),
            ];
            assert_eq!(
                daily_pick(&history, &BTreeMap::new(), "Trumpet", false, now(), 42),
                None,
                "non-finite cents ({bad}) must produce no candidate"
            );
        }
        let mut km = BTreeMap::new();
        km.insert("2:major".to_owned(), mastery(10, f32::NAN, 0));
        assert_eq!(daily_pick(&[], &km, "Trumpet", false, now(), 42), None);
    }
}
