//! Boss moments (#259 S1, epic #252): the payoff that turns a run of reps
//! into a short piece of music.
//!
//! After a few RV drills go well, [`maybe_compose_moment`] occasionally
//! assembles what was just practiced into a named musical moment — the most
//! recent well-drilled figure dealt through the keys the player just earned,
//! in the order they earned them. Composition only re-orders / re-keys
//! material the player already drilled (spec #259 §3): it never invents new
//! variation types, and the realized sequence comes from F1's `generate`, so
//! the moment plays, engraves, and grades exactly like the drills it honors.
//!
//! Everything here is **pure and deterministic**: time is injected as epoch
//! seconds (the same convention as `learner`), randomness comes only from the
//! caller's seed, and there is no I/O. The rate limiter is a single
//! `last_moment_at` timestamp compared against [`MomentConfig::window_secs`]
//! — one cadence source, mirroring the reveal loop (#253).
//!
//! v1 ships ONE moment shape — the keys tour (spec #259 §10 starts small):
//! same figure, rowed through the distinct keys of the recent well-drilled
//! material. A same-key medley (different figures in one key, a lesson's
//! natural payoff) is the tracked follow-up shape.

use serde::{Deserialize, Serialize};

use variations::{generate_in_window, FoldWindow, GeneratedSequence, VariationSpec};

use super::{key_signature_for, tonic_display_name};

/// A keys tour longer than this stops feeling like a moment and starts
/// feeling like another drill — cap the payoff at a short piece.
pub const MOMENT_MAX_KEYS: usize = 4;

/// The recent, already-scored RV material a moment can be built from,
/// **most-recent-first**. S3 assembles this from the session's drill results;
/// the composition core only reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillHistory {
    pub recent: Vec<DrilledItem>,
}

/// One drilled item: the F1 spec the player worked, the key/scale it was in
/// (F2's `(tonic, mode)` convention), and how it went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrilledItem {
    /// The F1 spec the user just drilled — carried so composition can re-key
    /// the exact figure, never a lookalike.
    pub spec: VariationSpec,
    /// Tonic pitch class the drill trained (0–11).
    pub tonic: u8,
    /// Material label, e.g. `"dorian"` / `"major triad"` (any casing).
    pub mode: String,
    /// 0..1, from `scoring` — the drill's graded accuracy.
    pub accuracy: f32,
    /// When the drill finished (Unix seconds, injected).
    pub completed_at_epoch_secs: i64,
}

/// Trigger + rate-limit tuning. Defaults are the spec's starting point (#259
/// §4); the feel — special, not spammy — is tuned in manual practice.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MomentConfig {
    /// How many well-drilled items must precede a moment.
    pub min_qualifying_drills: u32,
    /// An item qualifies only at/above this accuracy — a moment celebrates
    /// *well*-drilled material.
    pub min_accuracy: f32,
    /// At most one moment per this many seconds.
    pub window_secs: i64,
}

impl Default for MomentConfig {
    fn default() -> Self {
        Self {
            min_qualifying_drills: 3,
            min_accuracy: 0.7,
            window_secs: 600,
        }
    }
}

/// A composed boss moment: the realized F1 sequence (ticks for `ScoreView`,
/// `target_midi` for grading), the key/tempo the band locks to (S3), and a
/// stable `concept` for the F2 collection marker (S2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BossMoment {
    /// Human title, e.g. `"Major triad in 3 keys — F · G · C"`.
    pub label: String,
    /// The playable, gradable moment — F1's shape, consumed unchanged by the
    /// existing `ScoreView` adapter and scoring path.
    pub sequence: GeneratedSequence,
    /// Tonic pitch class the band plays in (the most recently drilled key).
    pub tonic: u8,
    /// Mode/scale label for the band's key, from the same drilled item.
    pub mode: String,
    /// Tempo the band locks to — the tempo the material was drilled at.
    pub tempo_bpm: f64,
    /// Stable collection key, e.g. `"keys-tour-major-triad-3"` — dedup key
    /// for the F2 "moment achieved" marker (S2).
    pub concept: String,
}

/// [`maybe_compose_moment_windowed`] under the default register window — the
/// voice/unknown-instrument path, same convention as F1's `generate`.
pub fn maybe_compose_moment(
    history: &DrillHistory,
    last_moment_at_epoch_secs: Option<i64>,
    now_epoch_secs: i64,
    cfg: &MomentConfig,
    seed: u64,
) -> Option<BossMoment> {
    maybe_compose_moment_windowed(
        history,
        last_moment_at_epoch_secs,
        now_epoch_secs,
        cfg,
        seed,
        FoldWindow::default(),
    )
}

/// Compose a boss moment from recent drilled material, or decline.
///
/// Returns `Some` only when ALL hold (spec #259 §4):
/// - the rate limit allows it — no prior moment, or
///   `now - last_moment_at >= window_secs` (the boundary fires);
/// - at least [`MomentConfig::min_qualifying_drills`] recent items are at/above
///   [`MomentConfig::min_accuracy`] (NaN never qualifies);
/// - the qualifying material spans ≥ 2 distinct tonics — the v1 keys-tour
///   shape needs keys to tour; an all-one-key history composes nothing yet
///   (the medley shape is the follow-up).
///
/// The moment: the most recent qualifying figure, dealt through up to
/// [`MOMENT_MAX_KEYS`] distinct drilled keys in the order they were earned
/// (oldest first, ending on the freshest key), unshuffled — a payoff run, not
/// another roulette. Pure and deterministic for a fixed
/// `(history, last_moment_at, now, cfg, seed, window)`.
pub fn maybe_compose_moment_windowed(
    history: &DrillHistory,
    last_moment_at_epoch_secs: Option<i64>,
    now_epoch_secs: i64,
    cfg: &MomentConfig,
    seed: u64,
    window: FoldWindow,
) -> Option<BossMoment> {
    if let Some(last) = last_moment_at_epoch_secs {
        if now_epoch_secs.saturating_sub(last) < cfg.window_secs {
            return None;
        }
    }

    // NaN accuracy never qualifies: `NaN >= x` is false by IEEE comparison.
    let qualifying: Vec<&DrilledItem> = history
        .recent
        .iter()
        .filter(|d| d.accuracy >= cfg.min_accuracy)
        .filter(|d| !d.spec.roots.is_empty())
        .collect();
    if (qualifying.len() as u32) < cfg.min_qualifying_drills {
        return None;
    }

    // Distinct tonics, most-recent-first, capped — then reversed so the tour
    // plays the keys in the order they were earned and ends on the freshest.
    let mut tour_tonics: Vec<u8> = Vec::new();
    for item in &qualifying {
        let pc = item.tonic % 12;
        if !tour_tonics.contains(&pc) {
            tour_tonics.push(pc);
            if tour_tonics.len() == MOMENT_MAX_KEYS {
                break;
            }
        }
    }
    if tour_tonics.len() < 2 {
        return None;
    }
    tour_tonics.reverse();

    // The most recent qualifying drill supplies the figure, register, rhythm,
    // and the key/mode the band plays in.
    let template = qualifying[0];
    let base_root = *template.spec.roots.first()?;

    let mut spec = template.spec.clone();
    spec.roots = tour_tonics
        .iter()
        .map(|&pc| nearest_root(base_root, pc))
        .collect();
    // A payoff plays in earned order — never reshuffled.
    spec.randomize_roots = false;

    let sequence = generate_in_window(&spec, seed, window);

    let figure = figure_name(&template.spec);
    // Each key spells to ITS OWN conventional signature (Bb major, not A# —
    // the #387 honesty rule), same derivation the engraving uses.
    let key_names: Vec<&str> = tour_tonics
        .iter()
        .map(|&pc| tonic_display_name(pc, key_signature_for(pc, &template.mode).fifths))
        .collect();
    let label = format!(
        "{figure} in {} keys — {}",
        tour_tonics.len(),
        key_names.join(" · ")
    );
    let concept = format!("keys-tour-{}-{}", slug(&figure), tour_tonics.len());

    Some(BossMoment {
        label,
        sequence,
        tonic: template.tonic % 12,
        mode: template.mode.clone(),
        tempo_bpm: template.spec.rhythm.tempo_bpm,
        concept,
    })
}

/// The MIDI root nearest `base` with pitch class `pc` — anchors every tour
/// stop to the FRESHEST drill's register (deliberate: a compact run through
/// the keys, not a jump to each key's own drilled octave). Octave-adjusts at
/// the MIDI rails so the pitch class is never wrong (the generator's fold
/// handles comfort).
fn nearest_root(base: u8, pc: u8) -> u8 {
    let mut diff = (i16::from(pc % 12) - i16::from(base % 12)).rem_euclid(12);
    if diff > 6 {
        diff -= 12;
    }
    let mut root = i16::from(base) + diff;
    if root < 0 {
        root += 12;
    } else if root > 127 {
        root -= 12;
    }
    root as u8
}

/// Display name for the drilled figure, mirroring F1's REAL precedence
/// (`figure_for`/`active_progression`): cell > progression > scale — with
/// degrees shadowing only INSIDE an active scale — > chord > interval > bare
/// root. Degrees without a scale are ignored by the generator, so naming
/// them here would label ticks the player never hears.
fn figure_name(spec: &VariationSpec) -> String {
    if spec.cell.as_ref().is_some_and(|c| !c.is_empty()) {
        return "Your phrase".to_owned();
    }
    if spec.progression.as_ref().is_some_and(|p| !p.is_empty()) {
        return "Your progression".to_owned();
    }
    if let Some(s) = spec.scale {
        if spec.degrees.as_ref().is_some_and(|d| !d.is_empty()) {
            return "Your pattern".to_owned();
        }
        return format!(
            "{} scale",
            capitalize_first(&s.scale.label().to_lowercase())
        );
    }
    if let Some(c) = spec.chord {
        return capitalize_first(&c.chord.label().to_lowercase());
    }
    if let Some(i) = spec.interval {
        return format!("Interval {}", i.semitones);
    }
    "Root notes".to_owned()
}

/// `"major triad"` → `"Major triad"` — sentence case for the card title.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Stable slug for the concept key: lowercase, runs of non-alphanumerics
/// collapse to single hyphens (`"Your phrase"` → `"your-phrase"`).
fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_hyphen = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_hyphen && !out.is_empty() {
                out.push('-');
            }
            pending_hyphen = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_hyphen = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use variations::{
        ArpeggioPattern, ChordModifier, ChordType, DirectionMode, RhythmSpec, ScaleModifier,
        ScalePattern, ScaleType,
    };

    /// A drilled major-triad arpeggio at `root`, graded `accuracy`.
    fn triad_item(tonic: u8, root: u8, accuracy: f32, completed_at: i64) -> DrilledItem {
        DrilledItem {
            spec: VariationSpec {
                roots: vec![root],
                cell: None,
                degrees: None,
                progression: None,
                scale: None,
                chord: Some(ChordModifier {
                    chord: ChordType::MajorTriad,
                    pattern: ArpeggioPattern::Ascending,
                    inversion: 0,
                    stacked: false,
                }),
                interval: None,
                enclosure: None,
                direction: DirectionMode::Forward,
                rhythm: RhythmSpec {
                    notes_per_beat: 2,
                    tempo_bpm: 92.0,
                    rest_beats_between_roots: 1.0,
                },
                randomize_roots: true,
            },
            tonic,
            mode: "major triad".to_owned(),
            accuracy,
            completed_at_epoch_secs: completed_at,
        }
    }

    /// Most-recent-first history: C (freshest), G, F — all well drilled.
    fn three_key_history() -> DrillHistory {
        DrillHistory {
            recent: vec![
                triad_item(0, 60, 0.9, 300),
                triad_item(7, 67, 0.85, 200),
                triad_item(5, 65, 0.8, 100),
            ],
        }
    }

    fn cfg() -> MomentConfig {
        MomentConfig::default()
    }

    // AC1: qualifying material + no prior moment → a moment composed from the
    // supplied specs — the drilled triad figure, re-keyed, nothing invented.
    #[test]
    fn composes_from_recent_material() {
        let moment = maybe_compose_moment(&three_key_history(), None, 1_000, &cfg(), 42)
            .expect("3 qualifying drills in 3 keys must compose a moment");

        assert!(
            !moment.sequence.target_midi.is_empty(),
            "moment must carry a gradable target"
        );
        // Earned order: F, G, then C (the freshest key) last.
        let root_pcs: Vec<u8> = moment.sequence.root_order.iter().map(|r| r % 12).collect();
        assert_eq!(root_pcs, vec![5, 7, 0], "tour plays keys in earned order");
        // Every segment spells the DRILLED figure (major triad) of its key —
        // 3 chord tones per key, root/third/fifth pitch classes, no extras.
        assert_eq!(moment.sequence.target_midi.len(), 9);
        for (i, &pc) in root_pcs.iter().enumerate() {
            let segment: Vec<u8> = moment.sequence.target_midi[i * 3..(i + 1) * 3]
                .iter()
                .map(|m| m % 12)
                .collect();
            assert_eq!(
                segment,
                vec![pc, (pc + 4) % 12, (pc + 7) % 12],
                "segment {i} must arpeggiate the drilled major triad of its key"
            );
        }
    }

    // AC2: label + concept present and concrete; key/tempo derived from the
    // most recent drilled material so the band can play it.
    #[test]
    fn moment_carries_label_concept_key_tempo() {
        let moment = maybe_compose_moment(&three_key_history(), None, 1_000, &cfg(), 42)
            .expect("moment composes");
        assert_eq!(moment.label, "Major triad in 3 keys — F · G · C");
        assert_eq!(moment.concept, "keys-tour-major-triad-3");
        assert_eq!(moment.tonic, 0, "band key = most recently drilled tonic");
        assert_eq!(moment.mode, "major triad");
        assert!(
            (moment.tempo_bpm - 92.0).abs() < f64::EPSILON,
            "band tempo = the drilled tempo, got {}",
            moment.tempo_bpm
        );
    }

    // AC3: too few items, or enough items but low accuracy → no moment.
    #[test]
    fn too_few_or_low_accuracy_returns_none() {
        let two = DrillHistory {
            recent: three_key_history().recent[..2].to_vec(),
        };
        assert_eq!(maybe_compose_moment(&two, None, 1_000, &cfg(), 42), None);

        let mut low = three_key_history();
        low.recent[1].accuracy = 0.69; // one under the 0.7 bar → only 2 qualify
        assert_eq!(maybe_compose_moment(&low, None, 1_000, &cfg(), 42), None);
    }

    // AC4: within the window, qualifying material stays rate-limited.
    #[test]
    fn rate_limited_within_window() {
        let now = 10_000;
        let last = now - (cfg().window_secs - 1); // one second short
        assert_eq!(
            maybe_compose_moment(&three_key_history(), Some(last), now, &cfg(), 42),
            None
        );
    }

    // AC5 + window boundary: fires with no prior moment, and at exactly the
    // window (>=, inclusive).
    #[test]
    fn fires_after_window_elapsed() {
        let now = 10_000;
        let at_boundary = now - cfg().window_secs;
        assert!(
            maybe_compose_moment(&three_key_history(), Some(at_boundary), now, &cfg(), 42)
                .is_some(),
            "now - last == window must fire (inclusive boundary)"
        );
        assert!(maybe_compose_moment(&three_key_history(), None, now, &cfg(), 42).is_some());
        // No prior moment fires regardless of the clock's origin — S3 must
        // not regress with a session-relative now of 0.
        assert!(maybe_compose_moment(&three_key_history(), None, 0, &cfg(), 42).is_some());
    }

    // The trigger's key-span boundary: exactly 2 distinct keys is enough to
    // tour (>= 2), pinned so the coherence rule can't silently drift to 3.
    #[test]
    fn two_distinct_keys_fire() {
        let history = DrillHistory {
            recent: vec![
                triad_item(0, 60, 0.9, 300),
                triad_item(7, 67, 0.85, 200),
                triad_item(0, 60, 0.8, 100),
            ],
        };
        let moment = maybe_compose_moment(&history, None, 1_000, &cfg(), 42)
            .expect("3 qualifying drills across 2 keys must compose");
        let root_pcs: Vec<u8> = moment.sequence.root_order.iter().map(|r| r % 12).collect();
        assert_eq!(root_pcs, vec![7, 0]);
        assert_eq!(moment.label, "Major triad in 2 keys — G · C");
    }

    // "At/above" means exactly min_accuracy qualifies (>=, inclusive).
    #[test]
    fn accuracy_exactly_at_bar_qualifies() {
        let mut history = three_key_history();
        history.recent[2].accuracy = 0.7;
        assert!(
            maybe_compose_moment(&history, None, 1_000, &cfg(), 42).is_some(),
            "an item at exactly the bar must count toward the threshold"
        );
    }

    // The config is plumbed, not decorative: non-default threshold, accuracy
    // bar, window, and a non-default drilled tempo all shape the outcome.
    #[test]
    fn non_default_config_is_honored() {
        let cfg = MomentConfig {
            min_qualifying_drills: 2,
            min_accuracy: 0.5,
            window_secs: 60,
        };
        let mut history = DrillHistory {
            recent: vec![triad_item(2, 62, 0.55, 200), triad_item(9, 69, 0.55, 100)],
        };
        history.recent[0].spec.rhythm.tempo_bpm = 132.0;
        let now = 10_000;
        // Two 0.55-accuracy drills clear THIS config (not the 3 × 0.7 default)…
        let moment = maybe_compose_moment(&history, Some(now - 60), now, &cfg, 42)
            .expect("2 drills at 0.55 clear this config");
        assert!(
            (moment.tempo_bpm - 132.0).abs() < f64::EPSILON,
            "tempo follows the drilled material, got {}",
            moment.tempo_bpm
        );
        // …and the 60-second window rate-limits, not the default 600.
        assert_eq!(
            maybe_compose_moment(&history, Some(now - 59), now, &cfg, 42),
            None
        );
    }

    // Degrees without a scale are ignored by the generator (F1 precedence) —
    // the label and concept must name what actually deals, never the inert
    // field (#387 display honesty; a wrong concept would pollute S2's
    // deduped collection forever).
    #[test]
    fn label_precedence_matches_the_generator() {
        let mut history = three_key_history();
        for item in &mut history.recent {
            item.spec.degrees = Some(vec![1, 3, 5]);
        }
        // Chord still set; degrees without a scale are inert → the triad deals.
        let moment = maybe_compose_moment(&history, None, 1_000, &cfg(), 42).unwrap();
        let first: Vec<u8> = moment.sequence.target_midi[..3]
            .iter()
            .map(|m| m % 12)
            .collect();
        assert_eq!(first, vec![5, 9, 0], "the triad deals; degrees are inert");
        assert_eq!(moment.label, "Major triad in 3 keys — F · G · C");
        // WITH a scale the degree pattern IS the audible figure and names itself.
        for item in &mut history.recent {
            item.spec.chord = None;
            item.spec.scale = Some(ScaleModifier {
                scale: ScaleType::Major,
                pattern: ScalePattern::Up,
            });
        }
        let moment = maybe_compose_moment(&history, None, 1_000, &cfg(), 42).unwrap();
        assert_eq!(moment.label, "Your pattern in 3 keys — F · G · C");
        assert_eq!(moment.concept, "keys-tour-your-pattern-3");
    }

    // The register anchor survives the MIDI rails with the pitch class intact
    // (without the rail wrap, base 2 → pc 9 would realize as `-3 as u8` = 253).
    #[test]
    fn nearest_root_holds_pitch_class_at_the_midi_rails() {
        assert_eq!(nearest_root(2, 9), 9, "low rail wraps up an octave");
        assert_eq!(nearest_root(126, 11), 119, "high rail wraps down an octave");
        assert_eq!(nearest_root(60, 6), 66, "the tritone tie resolves upward");
        assert_eq!(nearest_root(69, 4), 64, "plain nearest may step down");
    }

    // AC6: same inputs → identical moment, bit for bit.
    #[test]
    fn moment_is_deterministic() {
        let a = maybe_compose_moment(&three_key_history(), None, 1_000, &cfg(), 7).unwrap();
        let b = maybe_compose_moment(&three_key_history(), None, 1_000, &cfg(), 7).unwrap();
        assert_eq!(a, b);
    }

    // Edge: first run, nothing drilled yet.
    #[test]
    fn empty_history_returns_none() {
        let empty = DrillHistory { recent: vec![] };
        assert_eq!(maybe_compose_moment(&empty, None, 1_000, &cfg(), 42), None);
    }

    // Edge: exactly min_qualifying_drills qualifying items → fires (>=).
    #[test]
    fn exact_threshold_fires() {
        let mut history = three_key_history();
        // A fourth, unqualifying item must not be needed AND must not block.
        history.recent.push(triad_item(9, 69, 0.2, 50));
        assert_eq!(history.recent.len(), 4);
        let qualifying = history.recent.iter().filter(|d| d.accuracy >= 0.7).count();
        assert_eq!(qualifying, 3, "fixture: exactly the default threshold");
        assert!(maybe_compose_moment(&history, None, 1_000, &cfg(), 42).is_some());
    }

    // Edge (v1 coherence rule): all qualifying drills in ONE key — the keys
    // tour has nothing to tour; no moment until the medley shape lands.
    #[test]
    fn single_key_history_returns_none() {
        let history = DrillHistory {
            recent: vec![
                triad_item(0, 60, 0.9, 300),
                triad_item(0, 60, 0.85, 200),
                triad_item(12, 60, 0.8, 100), // tonic 12 ≡ pc 0 — same key
            ],
        };
        assert_eq!(
            maybe_compose_moment(&history, None, 1_000, &cfg(), 42),
            None
        );
    }

    // Edge: NaN accuracy never qualifies (mirrors learner's NaN hygiene).
    #[test]
    fn nan_accuracy_does_not_qualify() {
        let mut history = three_key_history();
        history.recent[2].accuracy = f32::NAN;
        assert_eq!(
            maybe_compose_moment(&history, None, 1_000, &cfg(), 42),
            None
        );
    }

    // Edge: an item whose spec lost its roots can't anchor a register — it
    // must not qualify (and must not panic).
    #[test]
    fn rootless_spec_does_not_qualify() {
        let mut history = three_key_history();
        history.recent[0].spec.roots.clear();
        // Only 2 items with roots remain → below the threshold.
        assert_eq!(
            maybe_compose_moment(&history, None, 1_000, &cfg(), 42),
            None
        );
    }

    // Edge: more distinct keys than the cap → the tour keeps the most recent
    // MOMENT_MAX_KEYS, still in earned order.
    #[test]
    fn tour_caps_at_max_keys_keeping_most_recent() {
        let history = DrillHistory {
            recent: vec![
                triad_item(0, 60, 0.9, 500),  // C, freshest
                triad_item(7, 67, 0.9, 400),  // G
                triad_item(5, 65, 0.9, 300),  // F
                triad_item(10, 58, 0.9, 200), // Bb
                triad_item(3, 63, 0.9, 100),  // Eb — oldest, over the cap
            ],
        };
        let moment = maybe_compose_moment(&history, None, 1_000, &cfg(), 42).unwrap();
        let root_pcs: Vec<u8> = moment.sequence.root_order.iter().map(|r| r % 12).collect();
        assert_eq!(
            root_pcs,
            vec![10, 5, 7, 0],
            "cap keeps the 4 most recent keys, played in earned order"
        );
        assert_eq!(
            moment.label, "Major triad in 4 keys — Bb · F · G · C",
            "Bb spells flat-side per its own signature (#387), never A#"
        );
    }

    // The tour stays in the drilled register: each key's root is realized
    // nearest the most recent drill's root, never a far octave.
    #[test]
    fn tour_roots_stay_near_the_drilled_register() {
        let moment = maybe_compose_moment(&three_key_history(), None, 1_000, &cfg(), 42).unwrap();
        // Template root is C4 (60): F → 65, G → 67... nearest-fold: F (pc 5)
        // is +5 → 65; G (pc 7) is +7 → wraps to −5 → 55.
        assert_eq!(moment.sequence.root_order, vec![65, 55, 60]);
    }

    // A duplicate key later in the window must not appear twice in the tour.
    #[test]
    fn duplicate_keys_dedupe_into_one_tour_stop() {
        let history = DrillHistory {
            recent: vec![
                triad_item(0, 60, 0.9, 400),
                triad_item(7, 67, 0.9, 300),
                triad_item(0, 60, 0.9, 200), // C again
                triad_item(5, 65, 0.9, 100),
            ],
        };
        let moment = maybe_compose_moment(&history, None, 1_000, &cfg(), 42).unwrap();
        let root_pcs: Vec<u8> = moment.sequence.root_order.iter().map(|r| r % 12).collect();
        assert_eq!(root_pcs, vec![5, 7, 0], "C appears once, tour ends on it");
        assert_eq!(moment.label, "Major triad in 3 keys — F · G · C");
    }

    // A scale figure names itself in the label and concept — the moment
    // honors what was actually drilled, not a hardcoded triad.
    #[test]
    fn scale_figure_names_label_and_concept() {
        let mut history = three_key_history();
        for item in &mut history.recent {
            item.spec.chord = None;
            item.spec.scale = Some(ScaleModifier {
                scale: ScaleType::Dorian,
                pattern: ScalePattern::Up,
            });
            item.mode = "dorian".to_owned();
        }
        let moment = maybe_compose_moment(&history, None, 1_000, &cfg(), 42).unwrap();
        // Dorian engraves flat-side (fifths −2 from C major) → flat spellings.
        assert_eq!(moment.label, "Dorian scale in 3 keys — F · G · C");
        assert_eq!(moment.concept, "keys-tour-dorian-scale-3");
        assert_eq!(moment.mode, "dorian");
    }

    // The composed spec never reshuffles: same seed or different seed, the
    // tour's key order is the earned order.
    #[test]
    fn tour_order_ignores_the_seed_shuffle() {
        let a = maybe_compose_moment(&three_key_history(), None, 1_000, &cfg(), 1).unwrap();
        let b = maybe_compose_moment(&three_key_history(), None, 1_000, &cfg(), 999).unwrap();
        assert_eq!(a.sequence.root_order, b.sequence.root_order);
    }
}
