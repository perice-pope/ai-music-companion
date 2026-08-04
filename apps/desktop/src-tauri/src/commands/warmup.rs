//! #257 S3: the daily warmup ritual's IPC — throw, grade + streak, badge read.
//!
//! First per-domain module of the #365 `commands.rs` split: the command
//! wrappers, their pure `_impl`s, the DTOs, and the boundary clock read
//! live together here.

use chrono::{DateTime, Offset};
use serde::{Deserialize, Serialize};
use tauri::State;

use brain::store::LOCAL_TASTE_PROFILE_USER_ID;

use super::{AppState, CommandError, LockRecover};

/// One thrown Daily Warmup Roulette challenge as the score view consumes it
/// (#257): the seed that dealt it (echoed back on completion so the grade
/// re-derives the exact target the player saw), F1's label, and the target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WarmupChallengeDto {
    pub seed: u64,
    /// e.g. `"F# Dorian scale · up-down · 72 BPM"`.
    pub label: String,
    /// MIDI numbers, in target order.
    pub target_notes: Vec<u8>,
}

/// The streak as the badge renders it (#257): the chain length and whether
/// today's warmup is already done.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreakDto {
    pub count: u32,
    pub completed_today: bool,
}

const SECS_PER_DAY: i64 = 86_400;

/// Instant → [`brain::learner::LocalDay`]: shift the Unix instant by the local
/// UTC offset, then floor-divide into days. `div_euclid` keeps pre-epoch
/// instants and negative offsets on the correct side of local midnight —
/// truncating division would round them toward the epoch.
fn local_day_from_instant(epoch_secs: i64, utc_offset_secs: i32) -> brain::learner::LocalDay {
    brain::learner::LocalDay((epoch_secs + i64::from(utc_offset_secs)).div_euclid(SECS_PER_DAY))
}

/// The calendar day `now` falls in for its own timezone. Generic over the
/// zone so the offset wiring is testable with a fixed offset — the runner's
/// TZ (usually UTC, offset 0) can't exercise a sign or drop mutation.
fn local_day_of<Tz: chrono::TimeZone>(now: &DateTime<Tz>) -> brain::learner::LocalDay {
    local_day_from_instant(now.timestamp(), now.offset().fix().local_minus_utc())
}

/// The feature's single clock read (#257 spec §4): "now" becomes a `LocalDay`
/// here at the IPC boundary, and the pure core only ever sees the ordinal.
fn today_local_day() -> brain::learner::LocalDay {
    local_day_of(&chrono::Local::now())
}

/// `completed_today` is a fact about `last_warmup`, not the streak tail: a
/// result recorded for a stale/backward day must not light the badge.
fn streak_dto(m: &brain::learner::LearnerModel, today: brain::learner::LocalDay) -> StreakDto {
    StreakDto {
        count: m.streak.count,
        completed_today: m.last_warmup.as_ref().is_some_and(|w| w.day == today),
    }
}

/// Pure implementation of `start_daily_warmup`: deal the throw for `seed`.
/// Read-only on the model — a throw the player abandons writes nothing.
pub fn start_daily_warmup_impl(seed: u64) -> WarmupChallengeDto {
    let challenge = brain::coach::roulette(seed);
    WarmupChallengeDto {
        seed: challenge.seed,
        label: challenge.sequence.label,
        target_notes: challenge.sequence.target_midi,
    }
}

/// The played stream is bounded before the O(target × played) grader (its
/// documented caller contract). No honest ~60 s warmup approaches this many
/// notes, and far below it the grader's precision cap has already ground the
/// grade to ~0 — truncation never changes a realistic score, it only bounds
/// the allocation an IPC payload can force.
const MAX_WARMUP_PLAYED_NOTES: usize = 4_096;

/// Pure implementation of `complete_daily_warmup`: re-deal the challenge from
/// its seed, grade the take, and fold the completion into the Learner Model —
/// the whole read-modify-write under the store lock (same shape as
/// `record_reveal_impl`) so two rapid completions can't lose an update. Any
/// finished warmup advances the streak regardless of grade (#257 AC10:
/// showing up is the ritual; the score only feeds `last_warmup`).
pub fn complete_daily_warmup_impl(
    state: &AppState,
    seed: u64,
    played_notes: &[u8],
    today: brain::learner::LocalDay,
) -> Result<StreakDto, CommandError> {
    let challenge = brain::coach::roulette(seed);
    let bounded = &played_notes[..played_notes.len().min(MAX_WARMUP_PLAYED_NOTES)];
    let score = brain::coach::score_warmup(&challenge.sequence.target_midi, bounded);
    let store = state.session_store.lock_or_recover();
    let current = store
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)?
        .unwrap_or_default();
    let next = brain::learner::apply_daily_completion(&current, today, score);
    store.upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &next)?;
    Ok(streak_dto(&next, today))
}

/// Pure implementation of `get_streak`, for the badge. Read-only.
pub fn get_streak_impl(
    state: &AppState,
    today: brain::learner::LocalDay,
) -> Result<StreakDto, CommandError> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)?
        .unwrap_or_default();
    Ok(streak_dto(&model, today))
}

#[tauri::command]
pub fn start_daily_warmup() -> WarmupChallengeDto {
    // Fresh seed per throw (spec §4); millis stay far below 2^53, so the
    // frontend echoes it back through a JS number losslessly.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    start_daily_warmup_impl(seed)
}

#[tauri::command]
pub fn complete_daily_warmup(
    state: State<'_, AppState>,
    seed: u64,
    played_notes: Vec<u8>,
) -> Result<StreakDto, String> {
    complete_daily_warmup_impl(&state, seed, &played_notes, today_local_day())
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn get_streak(state: State<'_, AppState>) -> Result<StreakDto, String> {
    get_streak_impl(&state, today_local_day()).map_err(|e| e.to_frontend())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::with_mocks()
    }

    /// #257 spec §7 "localday boundary": the instant→`LocalDay` conversion
    /// respects the local UTC offset — one instant lands on different calendar
    /// days under different offsets — and floors rather than truncates, so
    /// pre-epoch instants fall on the correct side of local midnight. Fails if
    /// the conversion drops the offset or rounds toward zero.
    #[test]
    fn localday_from_instant_uses_local_offset() {
        use brain::learner::LocalDay;
        // 1970-01-01 23:30 UTC…
        let late_evening = 23 * 3600 + 30 * 60;
        assert_eq!(local_day_from_instant(late_evening, 0), LocalDay(0));
        // …is already past local midnight in Tokyo (UTC+9): day 1.
        assert_eq!(local_day_from_instant(late_evening, 9 * 3600), LocalDay(1));
        // 1970-01-01 00:30 UTC is still 1969-12-31 in New York (UTC-5).
        assert_eq!(local_day_from_instant(30 * 60, -5 * 3600), LocalDay(-1));
        // Floor, not truncation: one second before the epoch is day -1.
        assert_eq!(local_day_from_instant(-1, 0), LocalDay(-1));
    }

    /// #257 S3: the wrapper wiring reads the offset off the instant itself —
    /// deterministic under fixed offsets regardless of the runner's TZ (a UTC
    /// runner, offset 0, cannot distinguish a dropped or sign-flipped offset).
    /// The bracket check then pins `today_local_day` to the same calendar
    /// arithmetic — it fails on a millis-for-seconds or UTC-only mutant.
    #[test]
    fn local_day_rides_the_instants_own_offset() {
        use brain::learner::LocalDay;
        use chrono::TimeZone;
        // 1970-01-01 23:30 UTC: next day in Tokyo, same day in UTC…
        let late_evening = 23 * 3600 + 30 * 60;
        let tokyo = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(
            local_day_of(&tokyo.timestamp_opt(late_evening, 0).unwrap()),
            LocalDay(1)
        );
        assert_eq!(
            local_day_of(&chrono::Utc.timestamp_opt(late_evening, 0).unwrap()),
            LocalDay(0)
        );
        // …and 00:30 UTC is still yesterday in New York.
        let new_york = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(
            local_day_of(&new_york.timestamp_opt(30 * 60, 0).unwrap()),
            LocalDay(-1)
        );

        // `today_local_day` agrees with chrono's own local calendar; sampled
        // twice in case the test straddles local midnight.
        let calendar_day = || {
            chrono::Local::now()
                .date_naive()
                .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
                .num_days()
        };
        let before = calendar_day();
        let got = today_local_day().0;
        let after = calendar_day();
        assert!(
            got == before || got == after,
            "today_local_day says {got}, local calendar says {before}..={after}"
        );
    }

    /// #257 S3: the DTO the score view renders is exactly the deal for its
    /// echoed seed — label and target match `roulette(seed)` — so a later
    /// `complete_daily_warmup(seed, …)` grades against the very target the
    /// player was shown. Fails if the mapping picks other fields or re-throws
    /// under a different seed.
    #[test]
    fn warmup_dto_echoes_the_seed_and_its_exact_deal() {
        let dto = start_daily_warmup_impl(42);
        let deal = brain::coach::roulette(42);
        assert_eq!(dto.seed, 42);
        assert_eq!(dto.label, deal.sequence.label);
        assert_eq!(dto.target_notes, deal.sequence.target_midi);
        assert!(!dto.target_notes.is_empty());
    }

    /// #257 AC9: completing the warmup through the command layer persists BOTH
    /// halves — `last_warmup { day, score }` and the streak transition — and
    /// `get_streak` then reports the new count with `completed_today == true`
    /// (and `false` again the next day). Fails if the load→grade→apply→write
    /// path drops either write, or if the badge read stops deriving from the
    /// persisted model.
    #[test]
    fn complete_warmup_writes_score_and_streak() {
        let s = state();
        let today = brain::learner::LocalDay(20_000);

        // Untouched model: badge dark, no chain.
        assert_eq!(
            get_streak_impl(&s, today).unwrap(),
            StreakDto {
                count: 0,
                completed_today: false
            }
        );

        // A perfect take: play exactly what seed 42 dealt.
        let target = brain::coach::roulette(42).sequence.target_midi;
        let dto = complete_daily_warmup_impl(&s, 42, &target, today).unwrap();
        assert_eq!(
            dto,
            StreakDto {
                count: 1,
                completed_today: true
            }
        );

        // Persisted, not just returned: re-read the model from the store.
        let model = s
            .session_store
            .lock_or_recover()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .expect("completion must write the model");
        let warmup = model.last_warmup.expect("last_warmup recorded");
        assert_eq!(warmup.day, today);
        assert!(
            (warmup.score - 1.0).abs() < 1e-6,
            "a perfect take grades 1.0, got {}",
            warmup.score
        );
        assert_eq!(model.streak.count, 1);
        assert_eq!(model.streak.last_completed_local_day, Some(today));

        // The badge read agrees with the persisted truth…
        assert_eq!(
            get_streak_impl(&s, today).unwrap(),
            StreakDto {
                count: 1,
                completed_today: true
            }
        );
        // …and the same chain reads not-yet-done tomorrow.
        assert_eq!(
            get_streak_impl(&s, brain::learner::LocalDay(20_001)).unwrap(),
            StreakDto {
                count: 1,
                completed_today: false
            }
        );
    }

    /// #257 AC10 at the boundary: a botched take (nothing played → grade 0.0)
    /// on the NEXT day still extends the chain — showing up is the ritual;
    /// only `last_warmup.score` differs. Also proves the second completion
    /// reads the first one back from the store (count 2 requires the
    /// persisted count 1). Fails if the command layer grows a minimum-score
    /// gate or stops round-tripping the model.
    #[test]
    fn zero_score_completion_still_advances_streak() {
        let s = state();
        let target = brain::coach::roulette(7).sequence.target_midi;
        complete_daily_warmup_impl(&s, 7, &target, brain::learner::LocalDay(100)).unwrap();

        let dto = complete_daily_warmup_impl(&s, 7, &[], brain::learner::LocalDay(101)).unwrap();
        assert_eq!(
            dto,
            StreakDto {
                count: 2,
                completed_today: true
            }
        );
        let model = s
            .session_store
            .lock_or_recover()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .unwrap();
        assert_eq!(model.last_warmup.unwrap().score, 0.0);
    }

    /// #257 spec §4: `completed_today` derives from `last_warmup.day`, NOT
    /// the streak tail. The two legitimately diverge (a sync-merged blob can
    /// carry a newer streak tail than its last recorded warmup) — a badge lit
    /// off the tail would claim today's ritual is done with no result for
    /// today. Fails if `streak_dto` compares `streak.last_completed_local_day`
    /// instead.
    #[test]
    fn completed_today_reads_last_warmup_not_the_streak_tail() {
        use brain::learner::LocalDay;
        let s = state();
        let mut model = brain::learner::LearnerModel::default();
        model.streak.count = 4;
        model.streak.last_completed_local_day = Some(LocalDay(100));
        model.last_warmup = Some(brain::learner::DailyWarmup {
            day: LocalDay(90),
            score: 0.8,
            extra: serde_json::Map::new(),
        });
        s.session_store
            .lock_or_recover()
            .upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &model)
            .unwrap();

        // On the streak-tail day: no warmup result for it → badge dark.
        assert_eq!(
            get_streak_impl(&s, LocalDay(100)).unwrap(),
            StreakDto {
                count: 4,
                completed_today: false
            }
        );
        // On the last warmup's own day: lit.
        assert_eq!(
            get_streak_impl(&s, LocalDay(90)).unwrap(),
            StreakDto {
                count: 4,
                completed_today: true
            }
        );
    }

    /// #257 S3: a runaway IPC payload (far beyond any honest ~60 s warmup)
    /// completes calmly and is truncated to exactly the first
    /// `MAX_WARMUP_PLAYED_NOTES` notes before grading. The take opens with
    /// the full target, so the expected grade is precisely full recall times
    /// the precision cap AT THE BOUND — an unbounded grader halves it (m is
    /// the whole payload) and a bound keeping the take's tail instead of its
    /// head drops recall to ~0. Either mutation fails the exact assert.
    #[test]
    fn runaway_played_stream_is_truncated_at_the_documented_bound() {
        let s = state();
        let target = brain::coach::roulette(3).sequence.target_midi;
        let n = target.len();
        let mut take = target;
        take.resize(2 * MAX_WARMUP_PLAYED_NOTES, 60);
        let dto = complete_daily_warmup_impl(&s, 3, &take, brain::learner::LocalDay(50)).unwrap();
        assert_eq!(
            dto,
            StreakDto {
                count: 1,
                completed_today: true
            }
        );
        let score = s
            .session_store
            .lock_or_recover()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .unwrap()
            .last_warmup
            .unwrap()
            .score;
        let expected = (n as f32 * 1.5 / MAX_WARMUP_PLAYED_NOTES as f32).min(1.0);
        assert!(
            (score - expected).abs() < 1e-6,
            "grade must be capped at the bound: expected {expected}, got {score}"
        );
    }
}
