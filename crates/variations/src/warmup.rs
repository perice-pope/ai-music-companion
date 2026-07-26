//! #257 S2 — the Daily Warmup Roulette: one seed → one key + one scale,
//! dealt by F1, plus the 0–1 grade of what came back.
//!
//! `roulette` is the one-tap throw: it derives a (root, scale) draw from the
//! seed alone — no RNG state, no I/O, no clock — builds the `VariationSpec`,
//! and deals it through [`generate`], so the same seed always throws the
//! identical challenge (reproducible from a recap, trivially testable).
//! `score_warmup` grades the played MIDI stream against the dealt
//! `target_midi` with the same rules as drill scoring: order-preserving,
//! octave-agnostic, and capped so noodling can't game it. Both are pure —
//! the command layer (S3) owns clocks, persistence, and the streak.

use serde::{Deserialize, Serialize};

use crate::{
    generate, splitmix64, DirectionMode, GeneratedSequence, RhythmSpec, ScaleModifier,
    ScalePattern, ScaleType, VariationSpec,
};

/// The roulette's scale wheel — the same "common scales, easy → exotic" set
/// the explore chip cycles (the coach's `EXPLORE_SCALES`), per the #257
/// catalog-scope call: start with what F1 already ships, expand later.
pub const WARMUP_SCALES: [ScaleType; 6] = [
    ScaleType::Major,
    ScaleType::MajorPentatonic,
    ScaleType::Mixolydian,
    ScaleType::Dorian,
    ScaleType::Blues,
    ScaleType::HarmonicMinor,
];

/// The roulette's key wheel starts here: the 12 chromatic roots from C4.
const WARMUP_ROOT_BASE: u8 = 60;

/// A calm warmup texture: one deliberate note per beat. The "~60 s" of the
/// daily ritual budgets the whole flow (throw → play → score, S4's pacing);
/// the dealt material is a single unhurried up-down pass.
const WARMUP_TEMPO_BPM: f64 = 72.0;

/// One thrown challenge: the spec that was dealt, the seed that dealt it
/// (echoed back so the grade is reproducible), and the F1 output whose
/// `target_midi` is the grading target and `label` the UI string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarmupChallenge {
    pub spec: VariationSpec,
    pub seed: u64,
    pub sequence: GeneratedSequence,
}

/// Throw the daily roulette: deterministically derive one key + scale from
/// `seed`, then deal it through F1. Pure — the same seed always yields the
/// same `(root, scale)` and the same sequence; the command layer supplies a
/// fresh seed per throw.
pub fn roulette(seed: u64) -> WarmupChallenge {
    // Two chained scramble steps give independent draws for key and scale —
    // a plain split of one step's bits would correlate them across the
    // small moduli (12 and 6 share a factor).
    let key_draw = splitmix64(seed);
    let scale_draw = splitmix64(key_draw);
    let root = WARMUP_ROOT_BASE + (key_draw % 12) as u8;
    let scale = WARMUP_SCALES[(scale_draw % WARMUP_SCALES.len() as u64) as usize];

    let spec = VariationSpec {
        roots: vec![root],
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
            tempo_bpm: WARMUP_TEMPO_BPM,
            ..RhythmSpec::default()
        },
        randomize_roots: false,
    };
    let sequence = generate(&spec, seed);
    WarmupChallenge {
        spec,
        seed,
        sequence,
    }
}

/// The drill-scoring precision cap (`brain::coach::score_drill`): the
/// alignment skips extra played notes for free, so recall alone is gameable —
/// a slow chromatic walk contains every target as a subsequence. Extras
/// beyond this multiple of the target length scale the grade down; a genuine
/// take with a few flubs is untouched.
const PLAYED_SLACK: f32 = 1.5;

/// Grade played notes against the dealt target: `1.0` for the target played
/// exactly (in any octave), strictly lower for wrong or missing notes, and
/// always in `0.0..=1.0`. Order-preserving longest-common-subsequence on
/// pitch CLASS — octave-agnostic for the same reason drill scoring is: a
/// voice warms up the right scale degrees wherever its range sits.
///
/// Cost is O(target × played) in time and memory: fine for a warmup's
/// ≤ ~15-note target, but the caller (S3's command layer) owns bounding
/// `played_midi` to session scale before handing it over.
pub fn score_warmup(target_midi: &[u8], played_midi: &[u8]) -> f32 {
    let n = target_midi.len();
    let m = played_midi.len();
    if n == 0 || m == 0 {
        return 0.0;
    }
    // LCS table over pitch-class matches, same alignment as `score_drill`.
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if target_midi[i] % 12 == played_midi[j] % 12 {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let recall = f32::from(dp[0][0]) / n as f32;
    let precision_factor = ((n as f32 * PLAYED_SLACK) / m as f32).min(1.0);
    (recall * precision_factor).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #257 AC7: the throw is a real F1 deal — non-empty target, exactly the
    /// sequence `generate` produces for the echoed (spec, seed) — and it is
    /// seed-stable: the same seed twice throws the identical challenge.
    /// Fails if the roulette forgets state into the draw (a clock, an RNG)
    /// or drifts from the generator it claims to wrap.
    #[test]
    fn roulette_matches_generate_and_is_seed_stable() {
        let a = roulette(42);
        let b = roulette(42);
        assert_eq!(a, b, "same seed must throw the identical challenge");
        assert_eq!(a.seed, 42, "the seed is echoed for reproducible grading");
        assert!(!a.sequence.target_midi.is_empty());
        assert_eq!(
            a.sequence,
            generate(&a.spec, a.seed),
            "the challenge IS the F1 deal for its own (spec, seed)"
        );
        // Golden pin (review r1): CROSS-version replay. Within-process
        // equality above can't catch an algorithm drift that changes what
        // every stored seed throws — this crate's stored-seed discipline
        // demands the draw itself stay bit-stable.
        assert_eq!(a.spec.roots, vec![61], "seed 42 draws C#4 — pinned");
        assert_eq!(
            a.spec.scale.map(|s| s.scale),
            Some(ScaleType::Mixolydian),
            "seed 42 draws Mixolydian — pinned"
        );
    }

    /// #257 AC7 (variety half): different seeds can land different keys AND
    /// different scales, every draw stays on the advertised wheels (12
    /// chromatic roots from C4 × WARMUP_SCALES), and each throw is a single
    /// root — the roulette throws ONE key a day, not the 12-key row.
    /// Fails if a draw index is biased off-wheel, the modulus wraps roots
    /// out of range, or — the review-r1 mutant — the two draws are
    /// CORRELATED (e.g. both from the same scramble step), which covers
    /// each wheel alone yet deals only 12 of the 72 (key, scale) pairs.
    #[test]
    fn roulette_draws_vary_and_stay_on_the_wheels() {
        let mut pairs = std::collections::BTreeSet::new();
        for seed in 0..1000u64 {
            let c = roulette(seed);
            assert_eq!(c.spec.roots.len(), 1, "one key per daily throw");
            let root = c.spec.roots[0];
            assert!(
                (WARMUP_ROOT_BASE..WARMUP_ROOT_BASE + 12).contains(&root),
                "root {root} off the C4 chromatic wheel"
            );
            let scale = c.spec.scale.expect("a warmup is always a scale").scale;
            assert!(
                WARMUP_SCALES.contains(&scale),
                "{scale:?} is not on the warmup wheel"
            );
            pairs.insert((root, format!("{scale:?}")));
        }
        // 1000 seeds deal every (key, scale) combination — anything less
        // means a draw is stuck or the two draws share bits.
        assert_eq!(
            pairs.len(),
            12 * WARMUP_SCALES.len(),
            "every (key, scale) pair must be reachable, got {}",
            pairs.len()
        );
    }

    /// The label is what S4 shows on the throw — it must name the drawn key
    /// and scale, not a placeholder. Fails if the spec is assembled with the
    /// wrong figure precedence (e.g. a leftover cell would relabel it).
    #[test]
    fn the_label_names_the_drawn_key_and_scale() {
        let c = roulette(7);
        let scale_label = c.spec.scale.unwrap().scale.label();
        assert!(
            c.sequence.label.contains(scale_label),
            "label {:?} must name {scale_label}",
            c.sequence.label
        );
        // The trailing space blocks the prefix trap: a C draw mislabeled
        // "C# …" must not pass a bare starts_with("C").
        let root_name = theory::pitch_class_name(c.spec.roots[0] % 12);
        assert!(
            c.sequence.label.starts_with(&format!("{root_name} ")),
            "label {:?} must lead with the key {root_name}",
            c.sequence.label
        );
    }

    /// #257 AC8: exact take → 1.0; the same take an octave down → still 1.0
    /// (octave-agnostic, the singer case); one wrong note → strictly below
    /// perfect; everything bounded to 0.0..=1.0. Each assertion dies to a
    /// different real bug: exact-MIDI comparison, a broken LCS, or an
    /// unclamped ratio.
    #[test]
    fn score_warmup_perfect_partial_bounds() {
        let target: Vec<u8> = roulette(3).sequence.target_midi;
        assert_eq!(score_warmup(&target, &target), 1.0);

        let octave_down: Vec<u8> = target.iter().map(|&m| m - 12).collect();
        assert_eq!(
            score_warmup(&target, &octave_down),
            1.0,
            "the right degrees in another register are a perfect warmup"
        );

        let mut one_flub = target.clone();
        one_flub[1] += 1; // a semitone miss on the 2nd note
        let flubbed = score_warmup(&target, &one_flub);
        assert!(flubbed < 1.0, "a wrong note must cost something: {flubbed}");
        assert!(flubbed > 0.5, "one flub must not zero the take: {flubbed}");

        // Order matters: the right pitch classes in the wrong order are NOT
        // the drill. (Not the dealt up-down figure here — that one is a
        // palindrome, so reversing it changes nothing; an ascending run is
        // the honest order probe.)
        let ascending: Vec<u8> = vec![60, 62, 64, 65, 67];
        let descending: Vec<u8> = ascending.iter().rev().copied().collect();
        let backwards = score_warmup(&ascending, &descending);
        assert!(
            backwards < 0.5,
            "playing the figure backwards is not a passing take: {backwards}"
        );
    }

    /// The precision cap: a chromatic crawl that CONTAINS the target as a
    /// subsequence (recall 1.0 by construction) must not grade near-perfect.
    /// Fails if the PLAYED_SLACK cap is dropped — the exact gaming
    /// `score_drill` documents.
    #[test]
    fn noodling_through_every_note_cannot_score_high() {
        let target: Vec<u8> = vec![60, 62, 64, 65, 67];
        // Walk every semitone from 55 to 70: hits all five targets in order
        // amid 16 notes of chroma.
        let crawl: Vec<u8> = (55..=70).collect();
        let s = score_warmup(&target, &crawl);
        let cap = 5.0 * PLAYED_SLACK / 16.0;
        assert!(
            (s - cap).abs() < 1e-6,
            "recall 1.0 over 16 played notes must scale by the cap ({cap}): {s}"
        );
    }

    /// Degenerate inputs stay bounded and honest: nothing played is 0.0
    /// (showing up means playing), an empty target grades 0.0 rather than
    /// dividing by zero, and garbage stays in range.
    #[test]
    fn score_warmup_degenerate_inputs() {
        let target = [60u8, 64, 67];
        assert_eq!(score_warmup(&target, &[]), 0.0);
        assert_eq!(score_warmup(&[], &target), 0.0);
        assert_eq!(score_warmup(&[], &[]), 0.0);
        let garbage: Vec<u8> = (0..255u16).map(|i| ((i * 7) % 128) as u8).collect();
        let s = score_warmup(&target, &garbage);
        assert!((0.0..=1.0).contains(&s), "out of bounds: {s}");
    }
}
