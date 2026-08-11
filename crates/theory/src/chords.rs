//! Jazz Ears T1a (#349 §5.1/§5.3): the chord vocabulary and the chroma
//! matcher. Pure pitch-class-set theory — no audio, no I/O, no labels
//! (display spelling lives in `brain`, next to the key-signature rules).
//!
//! The matcher scores every (root × template) against a normalized 12-bin
//! chroma by weighted overlap with a penalty for strong non-chord energy,
//! with subset tolerance so jazz SHELL voicings (a 13th chord without its
//! 5th) still match their quality.

/// A chord quality the ears can name (#349 §5.1). Ordered roughly by
/// harmonic complexity; the matcher prefers simpler qualities on ties so a
/// bare triad never gets labeled as a shell of something fancier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordQuality {
    Maj,
    Min,
    Dim,
    Aug,
    Sus2,
    Sus4,
    Maj6,
    Min6,
    Dom7,
    Maj7,
    Min7,
    Min7b5,
    Dim7,
    MinMaj7,
    Dom7Sus4,
    Add9,
    Dom9,
    Maj9,
    Min9,
    Dom13,
    Dom7b9,
    Dom7s9,
    Dom7s11,
}

impl ChordQuality {
    /// Interval set from the root, semitones 0..12.
    pub fn intervals(self) -> &'static [u8] {
        use ChordQuality::*;
        match self {
            Maj => &[0, 4, 7],
            Min => &[0, 3, 7],
            Dim => &[0, 3, 6],
            Aug => &[0, 4, 8],
            Sus2 => &[0, 2, 7],
            Sus4 => &[0, 5, 7],
            Maj6 => &[0, 4, 7, 9],
            Min6 => &[0, 3, 7, 9],
            Dom7 => &[0, 4, 7, 10],
            Maj7 => &[0, 4, 7, 11],
            Min7 => &[0, 3, 7, 10],
            Min7b5 => &[0, 3, 6, 10],
            Dim7 => &[0, 3, 6, 9],
            MinMaj7 => &[0, 3, 7, 11],
            Dom7Sus4 => &[0, 5, 7, 10],
            Add9 => &[0, 2, 4, 7],
            Dom9 => &[0, 2, 4, 7, 10],
            Maj9 => &[0, 2, 4, 7, 11],
            Min9 => &[0, 2, 3, 7, 10],
            Dom13 => &[0, 4, 7, 9, 10],
            Dom7b9 => &[0, 1, 4, 7, 10],
            Dom7s9 => &[0, 3, 4, 7, 10],
            Dom7s11 => &[0, 4, 6, 7, 10],
        }
    }

    /// Intervals a SHELL voicing may omit without losing the quality
    /// (#349 §5.1 jazz convention: the 5th is optional once a 7th defines
    /// the sound; a plain triad's 5th is NOT optional — three notes is
    /// already the minimum).
    fn optional(self) -> &'static [u8] {
        use ChordQuality::*;
        match self {
            Dom7 | Maj7 | Min7 | MinMaj7 | Dom7Sus4 | Dom9 | Maj9 | Min9 | Dom13 | Dom7b9
            | Dom7s9 => &[7],
            Dom7s11 => &[7, 4],
            _ => &[],
        }
    }

    /// The label suffix jazz players read: "" for major, "m", "7", "maj7"…
    /// (The root name — spelled per key signature — is prepended in `brain`.)
    pub fn suffix(self) -> &'static str {
        use ChordQuality::*;
        match self {
            Maj => "",
            Min => "m",
            Dim => "dim",
            Aug => "aug",
            Sus2 => "sus2",
            Sus4 => "sus4",
            Maj6 => "6",
            Min6 => "m6",
            Dom7 => "7",
            Maj7 => "maj7",
            Min7 => "m7",
            Min7b5 => "m7b5",
            Dim7 => "dim7",
            MinMaj7 => "mMaj7",
            Dom7Sus4 => "7sus4",
            Add9 => "add9",
            Dom9 => "9",
            Maj9 => "maj9",
            Min9 => "m9",
            Dom13 => "13",
            Dom7b9 => "7b9",
            Dom7s9 => "7#9",
            Dom7s11 => "7#11",
        }
    }

    /// Every quality, in tie-break order (simpler first).
    pub fn all() -> &'static [ChordQuality] {
        use ChordQuality::*;
        &[
            Maj, Min, Dim, Aug, Sus2, Sus4, Maj6, Min6, Dom7, Maj7, Min7, Min7b5, Dim7, MinMaj7,
            Dom7Sus4, Add9, Dom9, Maj9, Min9, Dom13, Dom7b9, Dom7s9, Dom7s11,
        ]
    }
}

/// A matched chord, before display spelling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChordMatch {
    pub root_pc: u8,
    pub quality: ChordQuality,
    /// The sounding bass pitch class when it's a chord tone other than the
    /// root — the slash in "C7/E". `None` = root position or bass unknown.
    pub bass_pc: Option<u8>,
    /// Match confidence 0..1 (template overlap minus non-chord penalty).
    pub confidence: f32,
}

/// Don't report a chord below this many clearly-sounding pitch classes —
/// one or two notes is a line, not a chord (the meter's job, not ours).
pub const MIN_CHORD_BINS: usize = 3;
/// A chroma bin counts as "sounding" above this fraction of the max bin.
const ACTIVE_BIN_RATIO: f32 = 0.25;
/// The ROOT bin must clear this (stricter) fraction of the max bin. A tone
/// barely over the noise gate can legitimately be a chord's extension, but
/// never its root — without this, a noise bin at ~0.3 lets a 5-note
/// template (e.g. Dom7b9 rooted on the noise) swallow a clean dim7 whose
/// every real tone it happens to contain.
// Round 5 (#415): 0.4 → 0.32, pinned by `real_g7_says_g7` — at 0.4 the
// real G7 fixture promotes "Bdim" (G7's rootless upper structure) in the
// early window where the folded G sits under 40% of max. A real chord's
// root can transiently be its weakest tone.
const ROOT_BIN_RATIO: f32 = 0.32;
/// Weight of the penalty for strong energy OUTSIDE the template.
const NON_CHORD_PENALTY: f32 = 0.8;
/// Score penalty per omitted optional tone, so an exact interpretation of
/// the sounding notes always beats a shell reading of the same notes
/// (Fsus4 must not be reported as C7sus4-with-the-5th-dropped).
const MISSING_TONE_PENALTY: f32 = 0.05;
/// Minimum score to report anything at all.
pub const MIN_CHORD_CONF: f32 = 0.5;
/// Absolute floor on the strongest bin before anything counts as sounding.
/// Smoothed silence decays toward zero but never reaches it; without this,
/// twelve near-equal noise bins read as "polyphony". Real playing lands
/// well above it (a sounding pitch class folds to ≳1.0).
const CHROMA_SILENCE_FLOOR: f32 = 0.1;

/// #382: any pitch class sounding at least this fraction of the strongest
/// bin is a REAL note, and a template that doesn't contain it is not the
/// chord being played — a loud C natural disqualifies Dmaj7 no matter how
/// well the other bins fit (the cluster→"Dmaj7" invention this run).
/// Round 2 (#411): raised 0.6 → 0.75 — real rooms spike resonance past
/// 60% of max on legitimately held chords, and the veto was killing every
/// candidate for a reading (her G7 flashing unrelated roots, C dropping
/// out). Played wrong notes read ≥ ~85%; the veto now waits for those.
/// PINNED (round 3): `c_triad_with_a_moderate_drone_never_drops` (4 s)
/// goes red at 0.6 and green at 0.75 — the constant is individually
/// pinned. Her recorded piano fixtures remain the eventual real-world
/// calibration.
const STRONG_OUTSIDER_RATIO: f32 = 0.75;

/// #382: an extension-class template tone (anything past root/3rd/5th in a
/// 4+-tone quality) must sound at least this fraction of the strongest bin
/// to count as PLAYED. Real-piano partial residue peaks around 25–45% of
/// max after log compression; a genuinely played tone reads ≥ ~90% at the
/// attack. Between the two, the honest call is "not played".
///
/// Round 2 (#411): this bar applies to PROMOTION only. A held piano chord
/// DECAYS — the seventh that read 90% at the attack reads 30% two seconds
/// later, and demanding promotion-grade evidence every reading made the
/// label decay with the note ("G7" → "G" mid-hold, the VA's flicker). The
/// INCUMBENT chord retains its tones at the audibility bar instead (see
/// [`best_match_retentive`] and [`RETAIN_FLOOR`]) — still sounding means
/// still named; truly released still switches.
const EXTENSION_MIN_RATIO: f32 = 0.55;

/// Round 5 (#415): see the maj7 note at the use site — the 3rd-partial
/// class needs a stronger claim than other extensions.
const MAJ7_MIN_RATIO: f32 = 0.7;

/// #382 interior dropout: an INCUMBENT's tone counts as still-audible above
/// this absolute level even when it fails the relative active bar. Root
/// doubling towers the max bin (LH C3 + RH C: max ≈ 1.5–1.8), and harmonic
/// subtraction eats the third in proportion to root energy, so the ringing
/// third reads 0.18–0.31 absolute — under 25%-of-max, over any honest
/// audibility floor. Calibrated between that band and a truly released
/// tone's residue, which falls under 0.15 within ~1 s (the g7-release
/// render) and keeps falling; retention therefore lets go later than the
/// relative bar would, a latency `releasing_the_seventh_releases_the_label`
/// bounds. Only [`best_match_retentive`]'s incumbent uses this — promotion
/// always pays full evidence — so an incumbent's bar is never HIGHER than
/// this value, whatever the max bin does. Must stay above
/// [`CHROMA_SILENCE_FLOOR`], or retention outlives the sound itself
/// (enforced below; the calibration band is pinned by the
/// towered-retention and dead-third unit tests: 0.14 < floor ≤ 0.19).
const RETAIN_FLOOR: f32 = 0.15;
const _: () = assert!(RETAIN_FLOOR > CHROMA_SILENCE_FLOOR);

/// Root/third/fifth pitch-class offsets — the tones ANY voicing carries.
/// (Folded 3 covers both the minor 3rd and the folded #9; that ambiguity is
/// inherent to pitch-class templates and errs toward accepting.)
const BASE_TRIAD_INTERVALS: [u8; 4] = [0, 3, 4, 7];

/// How many chroma bins are "sounding" under the matcher's own definition
/// (≥ [`ACTIVE_BIN_RATIO`] of the strongest bin). Lets callers tell honest
/// polyphony-without-a-name (≥ [`MIN_CHORD_BINS`] active, no confident
/// match) apart from single-line playing.
pub fn active_bin_count(chroma: &[f32; 12]) -> usize {
    let max_bin = chroma.iter().cloned().fold(0.0f32, f32::max);
    if max_bin < CHROMA_SILENCE_FLOOR {
        return 0;
    }
    chroma
        .iter()
        .filter(|&&v| v >= max_bin * ACTIVE_BIN_RATIO)
        .count()
}

/// Match a normalized 12-bin chroma (index = pitch class, C = 0) against
/// the vocabulary. `bass_pc` is the independently-detected lowest sounding
/// pitch class (from the monophonic tracker), used only to name inversions —
/// never to force the root.
///
/// Returns `None` when fewer than [`MIN_CHORD_BINS`] classes sound or no
/// template clears [`MIN_CHORD_CONF`] — the caller shows the honest
/// "hearing several notes…" state instead of a guess (#349 §5.3).
pub fn best_match(chroma: &[f32; 12], bass_pc: Option<u8>) -> Option<ChordMatch> {
    best_match_retentive(chroma, bass_pc, None)
}

/// [`best_match`] with retention hysteresis (#411): `incumbent` is the
/// currently displayed (root, quality), whose template gets the audibility
/// bar — the relative active bar or [`RETAIN_FLOOR`], whichever is lower —
/// for ALL its tones instead of the promotion bars, so a held chord keeps
/// its name while its decaying tones still sound at all, even under a
/// towered max bin (#382 interior dropout), and a genuinely released tone
/// still lets go. Promotion of any OTHER identity keeps full evidence
/// requirements.
pub fn best_match_retentive(
    chroma: &[f32; 12],
    bass_pc: Option<u8>,
    incumbent: Option<(u8, ChordQuality)>,
) -> Option<ChordMatch> {
    let d = denoise(chroma)?;
    // A decayed hold can thin to two relative-active bins while the chord
    // audibly rings (max ≳ 1.0) — the incumbent may still retain through
    // that, but nothing NEW can be named on fewer than MIN_CHORD_BINS.
    // Today every quality carries ≥ 3 required tones needing denoised
    // bins, so fresh minting on a thin chroma is already impossible by
    // template arithmetic — this gate (and its per-root twin in
    // [`root_is_candidate`]) is defense-in-depth that becomes LOAD-BEARING
    // the day a 2-required-tone quality (a "5" power chord, say) enters
    // the vocabulary.
    if d.active < MIN_CHORD_BINS && incumbent.is_none() {
        return None;
    }

    let mut best: Option<ChordMatch> = None;
    for root in 0u8..12 {
        let retained_root = incumbent.is_some_and(|(r, _)| r == root);
        if !root_is_candidate(&d, root, retained_root) {
            continue;
        }
        for &q in ChordQuality::all() {
            let retained = incumbent == Some((root, q));
            let Some(score) = score_quality(&d, root, q, retained) else {
                continue;
            };
            // Strictly-better wins; ties keep the earlier (simpler) quality
            // and the earlier root — deterministic and triad-favoring.
            // (#382 review note: an extra richer-must-win-by-a-margin rule
            // was tried here and proved inert — the extension evidence bar
            // and the outsider veto in [`score_quality`] already decide
            // every case the margin could have. Deleted rather than shipped
            // untestable.)
            if score > best.map_or(MIN_CHORD_CONF, |b| b.confidence) {
                best = Some(ChordMatch {
                    root_pc: root,
                    quality: q,
                    bass_pc: slash_bass(bass_pc, root, q.intervals()),
                    confidence: score.clamp(0.0, 1.0),
                });
            }
        }
    }
    best
}

/// The chroma after the silence gates and the denoise pass — what every
/// scoring seam reads. `raw` survives alongside `denoised` because the
/// incumbent's retention checks use an audibility floor BELOW the denoise
/// bar, and denoise zeroes exactly the bins retention must still see
/// (#382 interior dropout).
struct DenoisedChroma {
    raw: [f32; 12],
    denoised: [f32; 12],
    max_bin: f32,
    /// Denoised bins still sounding — the matcher's "active" count.
    active: usize,
    /// Total denoised energy — the score normalizer.
    total: f32,
}

/// Gate silence, then denoise with the same threshold that defines
/// "sounding": bins under it are analysis-noise floor (leakage, residual
/// harmonics), not evidence for or against any chord. Without the zeroing,
/// a clean four-note chord over a realistic floor scores below the
/// confidence gate. `None` = nothing is sounding at all.
fn denoise(chroma: &[f32; 12]) -> Option<DenoisedChroma> {
    let total: f32 = chroma.iter().sum();
    if total <= f32::EPSILON {
        return None;
    }
    let max_bin = chroma.iter().cloned().fold(0.0f32, f32::max);
    if max_bin < CHROMA_SILENCE_FLOOR {
        return None;
    }
    let mut denoised = *chroma;
    for v in denoised.iter_mut() {
        if *v < max_bin * ACTIVE_BIN_RATIO {
            *v = 0.0;
        }
    }
    Some(DenoisedChroma {
        raw: *chroma,
        max_bin,
        active: denoised.iter().filter(|&&v| v > 0.0).count(),
        total: denoised.iter().sum(),
        denoised,
    })
}

/// May `root` anchor a candidate at all? Promotion needs enough active
/// bins and a root clearing the (stricter) [`ROOT_BIN_RATIO`] bar.
///
/// #411 round 2 (review MF2): the ROOT bar also gets retention. Unison-
/// string beating dips the root fundamental below the promotion bar on a
/// held chord, which deleted the incumbent from candidacy entirely and
/// elected its upper structure (Cmaj7→Em) or dropped the label. Promotion
/// proved the root once; keeping the name only requires the root to stay
/// AUDIBLE — measured on the raw chroma against [`RETAIN_FLOOR`], because
/// the relative bar lies when another bin towers (#382 interior dropout).
fn root_is_candidate(d: &DenoisedChroma, root: u8, retained_root: bool) -> bool {
    if d.active < MIN_CHORD_BINS && !retained_root {
        return false;
    }
    if retained_root {
        d.raw[usize::from(root)] >= (d.max_bin * ACTIVE_BIN_RATIO).min(RETAIN_FLOOR)
    } else {
        d.denoised[usize::from(root)] >= d.max_bin * ROOT_BIN_RATIO
    }
}

/// Score one (root, quality) against the chroma: walk the template's
/// evidence bars, veto on a strong tone the template doesn't explain, and
/// net the non-chord penalty. `retained` marks the incumbent identity,
/// whose tones get the audibility bar instead of the promotion bars.
/// `None` = a required tone doesn't sound, or an outside tone disqualifies
/// the template.
fn score_quality(d: &DenoisedChroma, root: u8, q: ChordQuality, retained: bool) -> Option<f32> {
    let intervals = q.intervals();
    let optional = q.optional();
    let mut in_chord = 0.0f32;
    let mut missing = 0usize;
    let mut template_mask = [false; 12];
    for &iv in intervals {
        let pc = usize::from((root + iv) % 12);
        template_mask[pc] = true;
        // #382: extension tones need REAL evidence, not residue —
        // partial bleed lands exactly on 7th/9th/13th classes and
        // sits well above the plain active threshold after log
        // compression. Base-triad tones keep the active bar.
        let is_extension = intervals.len() > 3 && !BASE_TRIAD_INTERVALS.contains(&iv);
        // Round 5: the MAJOR SEVENTH (iv 11) sits exactly where the
        // bass/third's 3rd partial lands — the single most
        // residue-prone interval on real piano (a real C/E read
        // "Cmaj7/E" off the E's partials). Claiming maj7 takes a
        // genuinely prominent seventh.
        //
        // #382 interior dropout: the incumbent's tones — base triad
        // included — retain at an ABSOLUTE audibility floor when the
        // relative bar is towered (root doubling puts two hammers'
        // energy in one bin, and subtraction eats the third in
        // proportion to it — the ringing third reads 0.18–0.31 while
        // 25%-of-max asks ≥ 0.37). Measured on the raw chroma:
        // denoise zeroes exactly the bins retention must still see.
        let bar = if retained {
            (d.max_bin * ACTIVE_BIN_RATIO).min(RETAIN_FLOOR)
        } else if is_extension {
            if iv == 11 {
                d.max_bin * MAJ7_MIN_RATIO
            } else {
                d.max_bin * EXTENSION_MIN_RATIO
            }
        } else {
            d.max_bin * ACTIVE_BIN_RATIO
        };
        let heard = if retained { d.raw[pc] } else { d.denoised[pc] };
        // Required tones must actually sound (shells may drop the
        // optional ones — at a small cost, tallied below). Retained or
        // not, only the DENOISED value counts as evidence: retention
        // decides whether the name may stay, not how confident it reads.
        if heard < bar {
            if !optional.contains(&iv) {
                return None;
            }
            missing += 1;
            continue;
        }
        in_chord += d.denoised[pc];
    }
    let mut out_of_chord = 0.0f32;
    for pc in (0..12).filter(|&pc| !template_mask[pc]) {
        // #382: a strongly-sounding tone outside the template is a
        // played note this chord doesn't explain — veto, don't
        // just penalize (a cluster's loud C must kill Dmaj7).
        if d.denoised[pc] >= d.max_bin * STRONG_OUTSIDER_RATIO {
            return None;
        }
        out_of_chord += d.denoised[pc];
    }
    Some(
        (in_chord - NON_CHORD_PENALTY * out_of_chord) / d.total
            - MISSING_TONE_PENALTY * missing as f32,
    )
}

/// The slash in "C7/E": the independently-detected bass names an inversion
/// only when it's a chord tone other than the root — never forces the root.
fn slash_bass(bass_pc: Option<u8>, root: u8, intervals: &[u8]) -> Option<u8> {
    bass_pc
        .filter(|&b| b % 12 != root && intervals.iter().any(|&iv| (root + iv) % 12 == b % 12))
        .map(|b| b % 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a chroma with the given pitch classes sounding at strength 1
    /// (plus optional weaker extras).
    fn chroma_of(pcs: &[u8]) -> [f32; 12] {
        let mut c = [0.0f32; 12];
        for &pc in pcs {
            c[usize::from(pc % 12)] = 1.0;
        }
        c
    }

    /// #349 T1a AC1 (property): every quality at every root round-trips —
    /// feed the exact template, get the exact chord back. Fails if any
    /// template, the tie-break order, or the scorer regresses.
    #[test]
    fn every_template_at_every_root_round_trips() {
        for root in 0u8..12 {
            for &q in ChordQuality::all() {
                let pcs: Vec<u8> = q.intervals().iter().map(|&iv| (root + iv) % 12).collect();
                let m = best_match(&chroma_of(&pcs), None)
                    .unwrap_or_else(|| panic!("no match for {q:?} at root {root}"));
                // The exact pitch-class set can be quality-ambiguous
                // (Cmaj6 == Am7, Cdim7 == any of its rotations). Accept any
                // answer whose template reproduces the SAME set — that's
                // enharmonic-equivalence honesty, not a wrong answer.
                let matched: std::collections::BTreeSet<u8> = m
                    .quality
                    .intervals()
                    .iter()
                    .map(|&iv| (m.root_pc + iv) % 12)
                    .collect();
                let fed: std::collections::BTreeSet<u8> = pcs.iter().copied().collect();
                assert_eq!(matched, fed, "{q:?}@{root} matched {m:?}");
            }
        }
    }

    /// The canonical jazz ladder at C, unambiguous members asserted exactly.
    #[test]
    fn the_jazz_ladder_names_correctly() {
        let cases: &[(&[u8], ChordQuality)] = &[
            (&[0, 4, 7], ChordQuality::Maj),
            (&[0, 3, 7], ChordQuality::Min),
            (&[0, 4, 7, 10], ChordQuality::Dom7),
            (&[0, 4, 7, 11], ChordQuality::Maj7),
            (&[0, 3, 7, 10], ChordQuality::Min7),
            (&[0, 3, 6, 10], ChordQuality::Min7b5),
            (&[0, 3, 4, 7, 10], ChordQuality::Dom7s9),
            (&[0, 1, 4, 7, 10], ChordQuality::Dom7b9),
        ];
        for (pcs, want) in cases {
            let m = best_match(&chroma_of(pcs), None).expect("match");
            assert_eq!((m.root_pc, m.quality), (0, *want), "pcs {pcs:?}");
        }
    }

    /// #349 §5.2 subset tolerance: a 13th SHELL (no 5th) still reads as 13;
    /// a plain triad missing its 5th does NOT match (two notes are a line).
    #[test]
    fn shells_match_but_two_note_fragments_do_not() {
        // C13 shell: C E Bb A (3rd, 7th, 13th — no 5th).
        let m = best_match(&chroma_of(&[0, 4, 10, 9]), None).expect("shell matches");
        assert_eq!((m.root_pc, m.quality), (0, ChordQuality::Dom13));
        // C + G alone: no chord.
        assert!(best_match(&chroma_of(&[0, 7]), None).is_none());
    }

    /// Inversion honesty: bass = chord tone ≠ root → slash pc; bass = root
    /// or a non-chord tone → no slash.
    #[test]
    fn bass_names_inversions_only_when_it_is_a_chord_tone() {
        let c = chroma_of(&[0, 4, 7]);
        assert_eq!(best_match(&c, Some(4)).unwrap().bass_pc, Some(4)); // C/E
        assert_eq!(best_match(&c, Some(0)).unwrap().bass_pc, None); // root pos.
        assert_eq!(best_match(&c, Some(1)).unwrap().bass_pc, None); // Db ≠ tone
    }

    /// A marginal noise bin must not become another chord's ROOT: Cdim7
    /// with faint F noise reads Cdim7 (or a rotation), never F7b9 rooted on
    /// the noise. Fails if ROOT_BIN_RATIO is dropped to the active gate.
    #[test]
    fn a_noise_bin_cannot_be_a_root() {
        let mut c = chroma_of(&[0, 3, 6, 9]);
        c[5] = 0.3; // marginal noise — above ACTIVE, below ROOT threshold
        let m = best_match(&c, None).expect("dim7 still matches");
        assert!(
            m.root_pc.is_multiple_of(3) && m.quality == ChordQuality::Dim7,
            "must stay a dim7 rotation, got {m:?}"
        );
    }

    /// The non-chord penalty is load-bearing for honesty: a clean triad
    /// plus TWO strong foreign bins sits at ~0.37 with the penalty (honest
    /// None) but ~0.65 without it (a lying "C"). This fixture lives in the
    /// window where only the penalty separates the two — it fails if
    /// NON_CHORD_PENALTY is zeroed (the mush test below does not: that one
    /// stays under the gate from /total dilution alone).
    #[test]
    fn strong_foreign_bins_force_an_honest_refusal() {
        // F AND F# both strong over a C triad — the 4 and the #4 together
        // are unnameable, and (unlike most foreign pairs in this dense
        // template space) no template absorbs {0,4,5,6,7}.
        let mut c = chroma_of(&[0, 4, 7]);
        c[5] = 0.8;
        c[6] = 0.8;
        assert!(
            best_match(&c, None).is_none(),
            "triad + 2 strong foreign bins must refuse a label: {:?}",
            best_match(&c, None)
        );
    }

    /// Denoise is pinned IN-CRATE (not only via ears' rendered fixtures): a
    /// four-note chord over a sub-threshold noise floor must still clear
    /// the confidence gate. Fails if the denoise zeroing is removed.
    #[test]
    fn a_noise_floor_does_not_sink_a_clean_four_note_chord() {
        let mut c = chroma_of(&[0, 4, 7, 10]);
        for pc in [1usize, 2, 3, 5, 6, 8, 9, 11] {
            c[pc] = 0.2; // below ACTIVE_BIN_RATIO of max — pure floor
        }
        let m = best_match(&c, None).expect("C7 must match over a floor");
        assert_eq!((m.root_pc, m.quality), (0, ChordQuality::Dom7));
    }

    /// The floor is not evidence AGAINST the chord either: sub-threshold
    /// bins must leave confidence EXACTLY where the clean reading puts it,
    /// because denoise zeroes them out of both the penalty sum and the
    /// score normalizer. Fails if the normalizer reverts to the raw total
    /// (the floor would dilute every score) — a regression the
    /// gate-clearing test above cannot see, since 0.71 still clears 0.5.
    #[test]
    fn a_noise_floor_does_not_dilute_confidence() {
        let clean = chroma_of(&[0, 4, 7, 10]);
        let clean_conf = best_match(&clean, None).expect("clean C7").confidence;
        let mut floored = clean;
        for pc in [1usize, 2, 3, 5, 6, 8, 9, 11] {
            floored[pc] = 0.2; // below ACTIVE_BIN_RATIO of max — pure floor
        }
        let m = best_match(&floored, None).expect("floored C7");
        assert_eq!((m.root_pc, m.quality), (0, ChordQuality::Dom7));
        assert_eq!(
            m.confidence, clean_conf,
            "sub-threshold floor must not move confidence at all"
        );
    }

    /// The 3.16 s reading of the real c-major-over-c3 fixture (#382
    /// interior dropout): the doubled root towers the max bin to 1.83, and
    /// harmonic subtraction has eaten the still-ringing third down to 0.19
    /// — under 25%-of-max (0.46), over the audibility floor. Fresh
    /// matching honestly refuses (a new "C" claim needs a heard third),
    /// but the INCUMBENT C major must retain. Fails if RETAIN_FLOOR is
    /// removed or retention reads the denoised chroma (denoise zeroes the
    /// 0.19 bin).
    #[test]
    fn a_towered_root_does_not_starve_retention() {
        let mut c = [0.0f32; 12];
        c[0] = 1.83; // C — LH root + RH root, two hammers
        c[4] = 0.19; // E — ringing third, post-subtraction
        c[7] = 0.94; // G
        c[11] = 0.60; // B — root's maj7-region partial residue
        c[10] = 0.42; // A#
        c[1] = 0.27; // C# — leakage
        assert!(
            best_match(&c, None).is_none(),
            "a FRESH match must still refuse — the third is below promotion evidence"
        );
        let m = best_match_retentive(&c, None, Some((0, ChordQuality::Maj)))
            .expect("the incumbent C major must retain through the towered reading");
        assert_eq!((m.root_pc, m.quality), (0, ChordQuality::Maj));
    }

    /// Retention must not outlive the tone: the same towered shape with the
    /// third truly gone lets go — no template matches, so the tracker's
    /// clear dwell can run. The residue sits ONE notch under the floor
    /// (0.14 vs 0.15 — the g7-release calibration: a released tone falls
    /// under the floor within ~1 s and keeps falling), so this red-lines
    /// any RETAIN_FLOOR ≤ 0.14; the towered-retention test above red-lines
    /// any ≥ 0.20. Together they pin the calibration band around 0.15.
    #[test]
    fn retention_lets_go_when_the_third_truly_dies() {
        let mut c = [0.0f32; 12];
        c[0] = 1.83;
        c[4] = 0.14; // E released — residue just under the audibility floor
        c[7] = 0.94;
        c[11] = 0.60;
        c[10] = 0.42;
        c[1] = 0.27;
        assert!(
            best_match_retentive(&c, None, Some((0, ChordQuality::Maj))).is_none(),
            "a dead third must release the incumbent"
        );
    }

    /// The other retained tone class (#411 review MF2): unison-string
    /// beating dips the incumbent's ROOT below the relative bar while
    /// another chord tone towers. Root raw 0.18 under a 1.5 max asks
    /// ≥ 0.375 relative — fresh matching refuses (2 active bins), but the
    /// incumbent's root retains at the audibility floor. Fails if the
    /// root retention branch reverts to the denoised relative bar.
    #[test]
    fn a_beating_root_dip_does_not_unseat_the_incumbent() {
        let mut c = [0.0f32; 12];
        c[0] = 0.18; // C — root fundamental in a beat trough
        c[4] = 1.5; // E — towering third
        c[7] = 0.5; // G
        assert!(
            best_match(&c, None).is_none(),
            "a FRESH match must refuse — the root is below promotion evidence"
        );
        let m = best_match_retentive(&c, None, Some((0, ChordQuality::Maj)))
            .expect("the incumbent survives a root beat trough");
        assert_eq!((m.root_pc, m.quality), (0, ChordQuality::Maj));
    }

    /// The decayed-hold tail (#382): the take thins to TWO relative-active
    /// bins (C and G) while the chord still audibly rings — the 4.64 s
    /// reading. The incumbent may retain through it (its third is at 0.24
    /// absolute), but the same chroma with no incumbent must stay
    /// unnamed. Fails if the MIN_CHORD_BINS gate stops exempting the
    /// incumbent. (The fresh-refusal half holds by template arithmetic
    /// today — every quality carries ≥ 3 required tones — and is asserted
    /// as documentation; the gate itself is defense-in-depth, see its
    /// comment.)
    #[test]
    fn a_thinned_hold_retains_but_cannot_mint() {
        let mut c = [0.0f32; 12];
        c[0] = 1.02;
        c[4] = 0.24; // below 25% of max — not "active"
        c[7] = 0.82;
        c[11] = 0.23;
        c[1] = 0.20;
        let m = best_match_retentive(&c, None, Some((0, ChordQuality::Maj)))
            .expect("the incumbent retains through the thinned tail");
        assert_eq!((m.root_pc, m.quality), (0, ChordQuality::Maj));
        assert!(
            best_match(&c, None).is_none(),
            "two active bins must not name a fresh chord"
        );
    }

    /// Non-chord energy is punished: a triad drowned in chromatic mush must
    /// not report confidently. (Kept for the saturated case; the fixture
    /// above pins the penalty itself.)
    #[test]
    fn chromatic_mush_does_not_read_as_a_confident_triad() {
        let mut c = chroma_of(&[0, 4, 7]);
        for pc in [1usize, 2, 3, 5, 6, 8, 9, 10, 11] {
            c[pc] = 0.8;
        }
        assert!(
            best_match(&c, None).is_none(),
            "mush must fall below the confidence gate"
        );
    }
}
