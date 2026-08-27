//! #349 T4b (core) — group-level ensemble hearing from one mic.
//!
//! A band or section plays into a single room mic; this module says how the
//! GROUP sounded. Per the settled ensemble decision (decisions log
//! 2026-04-20): one mic cannot attribute anything to an individual player,
//! so every number here describes "the band" — never "Sarah". Surfaces that
//! render this report must keep that framing.
//!
//! What it measures (spec #349 §T4b), each behind an evidence gate that
//! yields `None` rather than a fabricated verdict:
//! - **Togetherness** — how tight the group's shared attacks are: onsets
//!   landing within a short window are one ensemble attack, and the spread
//!   (population std-dev) inside those clusters scores 0..1.
//! - **Tempo per section + spread** — the ensemble pulse per labeled span
//!   of the take, and which section rushed or dragged relative to the rest.
//!   The pulse is estimated over ensemble ATTACKS (cluster leading edges),
//!   never raw onsets — on raw onsets the intra-attack scatter gaps poison
//!   the median IOI. Section tempos are mix-pulse estimates: comparable
//!   within one take, not calibrated metronome BPM.
//! - **Balance trend** — band-level dynamics direction across the take.
//! - **Group chord verdict** — when expected material is loaded, the T2
//!   judge ([`ChordDrillJudge`]) grades the MIX's promoted chord readings
//!   against it: the ensemble sound is the player.
//!
//! Honest limits, stated rather than papered over:
//! - Two players striking inside the onset detector's resolution fuse into
//!   ONE onset, so a perfectly tight band produces singleton clusters —
//!   indistinguishable from a soloist. Togetherness therefore reports only
//!   when enough *scattered* attacks exist to measure; silence otherwise.
//! - Subdivisions faster than the cluster window (16ths above ~187 BPM at
//!   the default 80 ms) would read as scatter; ensemble section material
//!   sits well below that, and the window is a config knob.
//! - A section that switches subdivision (eighths after quarters) doubles
//!   its attack density without anyone rushing. A candidate outlier whose
//!   pulse sits near an integer multiple (or divisor) of the median is
//!   therefore treated as a subdivision change and never accused — the
//!   spread still reports the raw numbers.
//!
//! Pure and deterministic: no clock, no audio. This runs at recap time,
//! not on the audio thread — allocation is fine here.

use serde::{Deserialize, Serialize};

use crate::chord_judge::{chord_drill_accuracy, ChordDrillJudge, HeardChord};
use crate::follower::NoteVerdict;
use variations::ChordTarget;

/// Tuning constants for the group-level analysis. Defaults are the shipped
/// calibration; every gate that could accuse the band is deliberately
/// generous (coach, don't judge — an outlier call must be earned).
/// `serde(default)` so a future partially-persisted config falls back to
/// the shipped values instead of hard-failing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnsembleConfig {
    /// Onsets within this many seconds of a cluster's first onset count as
    /// the same ensemble attack.
    pub cluster_window_secs: f64,
    /// Mean intra-cluster spread at or below this scores togetherness 1.0.
    pub tight_spread_secs: f64,
    /// Mean intra-cluster spread at or above this scores togetherness 0.0.
    pub loose_spread_secs: f64,
    /// Fewer scattered (≥2-onset) clusters than this → togetherness `None`:
    /// not enough measurable scatter to speak.
    pub min_scattered_clusters: usize,
    /// A section needs at least this many ensemble attacks before its
    /// pulse estimate is trusted with a tempo.
    pub min_section_attacks: usize,
    /// A section must deviate from the median section tempo by more than
    /// this many BPM to be named as rushing or dragging.
    pub outlier_margin_bpm: f32,
    /// Fewer amplitude samples than this → balance trend `None`.
    pub min_balance_samples: usize,
    /// Relative loudness change between the take's halves that counts as a
    /// direction (0.15 = ±15%); anything inside reads Steady.
    pub balance_trend_ratio: f32,
}

impl Default for EnsembleConfig {
    fn default() -> Self {
        Self {
            cluster_window_secs: 0.08,
            tight_spread_secs: 0.010,
            loose_spread_secs: 0.040,
            min_scattered_clusters: 4,
            min_section_attacks: 8,
            outlier_margin_bpm: 8.0,
            min_balance_samples: 8,
            balance_trend_ratio: 0.15,
        }
    }
}

/// A labeled span of the take ("the opening", "the bridge"). Boundaries in
/// seconds from session start; half-open `[start, end)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub label: String,
    pub start_secs: f64,
    pub end_secs: f64,
}

/// Split a take into labeled thirds — the default sectioning when no score
/// provides real structure. Empty for a degenerate duration.
pub fn default_sections(duration_secs: f64) -> Vec<Section> {
    if duration_secs <= 0.0 || !duration_secs.is_finite() {
        return Vec::new();
    }
    let third = duration_secs / 3.0;
    ["the opening", "the middle", "the ending"]
        .iter()
        .enumerate()
        .map(|(i, label)| Section {
            label: (*label).to_string(),
            start_secs: i as f64 * third,
            end_secs: if i == 2 {
                // The last section closes the take just past its end, so
                // the final onset survives the half-open interval.
                duration_secs.next_up()
            } else {
                (i as f64 + 1.0) * third
            },
        })
        .collect()
}

/// How tight the group's shared attacks are, 0..1, with the evidence that
/// earned the number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Togetherness {
    /// 1.0 at/below the tight spread, 0.0 at/above the loose spread,
    /// linear between.
    pub score: f32,
    /// Mean intra-cluster spread across scattered clusters, in ms.
    pub mean_spread_ms: f32,
    /// Clusters with ≥2 onsets — the ones that carried evidence.
    pub scattered_clusters: u32,
    /// All ensemble attacks heard, singletons included.
    pub total_clusters: u32,
}

/// The ensemble pulse of one section. `tempo_bpm` is `None` when the
/// section held too few attacks to trust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionTempo {
    pub label: String,
    pub tempo_bpm: Option<f32>,
    /// Ensemble attacks (clusters) whose leading edge fell in the section.
    pub attack_count: u32,
}

// Both enums serialize snake_case to match the embedded `Verdict`
// ("hit"/"near"/"missed") — one casing per report on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TempoDirection {
    Rushed,
    Dragged,
}

/// The one section (if any) that earned a tempo accusation: the largest
/// deviation from the median section tempo, beyond the config margin.
/// Named only when at least three sections carry a tempo — with two, the
/// midpoint median makes rushed-vs-dragged an arbitrary tie-break, not a
/// verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TempoOutlier {
    /// A labeled TIME SPAN of the take (`Section.label`) — never an
    /// instrument section. Surfaces must render it so it cannot read as
    /// calling out the trombones (the group-only wording rule).
    pub section: String,
    pub direction: TempoDirection,
    /// Signed BPM distance from the median section tempo. The direction is
    /// the trustworthy signal; the magnitude is attack-density arithmetic
    /// over a merged multi-player train, not a metronome reading.
    pub delta_bpm: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceDirection {
    Rising,
    Falling,
    Steady,
}

/// Band-level dynamics across the take: mean amplitude of the first half
/// vs the second.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BalanceTrend {
    pub direction: BalanceDirection,
    pub early_mean: f32,
    pub late_mean: f32,
}

/// The mix's chord readings graded against the expected material — the
/// ensemble sound judged as one player, on the T2 rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupChordVerdict {
    pub verdicts: Vec<NoteVerdict>,
    /// Hit = 1, Near = ½, Missed = 0, over ALL targets.
    pub accuracy: f32,
}

/// Everything T4b can honestly say about a take. Every field is gated on
/// its own evidence; an empty room yields a report of `None`s, never a
/// fabricated verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnsembleReport {
    pub togetherness: Option<Togetherness>,
    /// One entry per input section, in input order, tempo'd or not.
    pub section_tempos: Vec<SectionTempo>,
    /// Max − min over the sections that earned a tempo; `None` below two.
    pub tempo_spread_bpm: Option<f32>,
    pub tempo_outlier: Option<TempoOutlier>,
    pub balance: Option<BalanceTrend>,
    /// Present only when expected material was provided.
    pub chords: Option<GroupChordVerdict>,
}

/// The raw session evidence the report is computed from. All slices may be
/// unsorted; the analysis sorts local copies.
#[derive(Debug, Clone, Copy)]
pub struct EnsembleInput<'a> {
    /// Onset timestamps of the MIX, seconds from session start.
    pub onsets_secs: &'a [f64],
    /// `(timestamp_secs, amplitude)` samples of the mix.
    pub amplitude_samples: &'a [(f64, f32)],
    /// Promoted chord changes heard over the take (the session's
    /// edge-triggered chord buffer).
    pub heard_chords: &'a [HeardChord],
    /// Expected material, when a score/drill is loaded.
    pub chord_targets: Option<&'a [ChordTarget]>,
    /// Labeled spans for per-section tempo. Empty → no tempo analysis.
    pub sections: &'a [Section],
}

/// Analyze one ensemble take, group-level only.
pub fn analyze_ensemble(input: &EnsembleInput, config: &EnsembleConfig) -> EnsembleReport {
    // Non-finite timestamps would scramble a partial-ord sort and poison
    // every downstream gap; they carry no musical meaning — drop them.
    let mut onsets: Vec<f64> = input
        .onsets_secs
        .iter()
        .copied()
        .filter(|t| t.is_finite())
        .collect();
    onsets.sort_by(f64::total_cmp);
    let clusters = cluster_attacks(&onsets, config.cluster_window_secs);

    let section_tempos = section_tempos(&clusters, input.sections, config);
    let (tempo_spread_bpm, tempo_outlier) = tempo_spread(&section_tempos, config);

    EnsembleReport {
        togetherness: togetherness(&clusters, config),
        section_tempos,
        tempo_spread_bpm,
        tempo_outlier,
        balance: balance_trend(input.amplitude_samples, config),
        chords: input
            .chord_targets
            .map(|targets| judge_group_chords(targets, input.heard_chords)),
    }
}

/// Grade the mix's chord changes against expected material on the T2
/// rules. Public seam so a score-loaded ensemble session can grade without
/// rerunning the whole report.
pub fn judge_group_chords(targets: &[ChordTarget], heard: &[HeardChord]) -> GroupChordVerdict {
    let mut judge = ChordDrillJudge::new(targets.to_vec());
    let mut verdicts = Vec::new();
    for &h in heard {
        verdicts.extend(judge.observe(h));
    }
    verdicts.extend(judge.finish());
    let accuracy = chord_drill_accuracy(&verdicts, targets.len());
    GroupChordVerdict { verdicts, accuracy }
}

/// One ensemble attack: a run of onsets within the cluster window of its
/// first onset.
struct Attack {
    /// The attack's leading edge — the cluster's first onset.
    start_secs: f64,
    onset_count: usize,
    /// Population std-dev of member onset times; 0.0 for a singleton.
    spread_secs: f64,
}

fn cluster_attacks(sorted_onsets: &[f64], window_secs: f64) -> Vec<Attack> {
    let mut attacks = Vec::new();
    let mut i = 0;
    while i < sorted_onsets.len() {
        let start = sorted_onsets[i];
        let mut end = i + 1;
        while end < sorted_onsets.len() && sorted_onsets[end] - start <= window_secs {
            end += 1;
        }
        let members = &sorted_onsets[i..end];
        attacks.push(Attack {
            start_secs: start,
            onset_count: members.len(),
            spread_secs: if members.len() >= 2 {
                population_std_dev(members)
            } else {
                0.0
            },
        });
        i = end;
    }
    attacks
}

/// Score the scatter across the attacks that carried any.
fn togetherness(attacks: &[Attack], config: &EnsembleConfig) -> Option<Togetherness> {
    let spreads: Vec<f64> = attacks
        .iter()
        .filter(|a| a.onset_count >= 2)
        .map(|a| a.spread_secs)
        .collect();
    if spreads.len() < config.min_scattered_clusters {
        return None;
    }

    let mean_spread = spreads.iter().sum::<f64>() / spreads.len() as f64;
    let span = config.loose_spread_secs - config.tight_spread_secs;
    let score = if span <= 0.0 {
        // Degenerate config: a single threshold — at-or-under is tight.
        if mean_spread <= config.tight_spread_secs {
            1.0
        } else {
            0.0
        }
    } else {
        (1.0 - ((mean_spread - config.tight_spread_secs) / span).clamp(0.0, 1.0)) as f32
    };

    Some(Togetherness {
        score,
        mean_spread_ms: (mean_spread * 1000.0) as f32,
        scattered_clusters: spreads.len() as u32,
        total_clusters: attacks.len() as u32,
    })
}

fn population_std_dev(xs: &[f64]) -> f64 {
    let mu = xs.iter().sum::<f64>() / xs.len() as f64;
    let var = xs.iter().map(|&x| (x - mu).powi(2)).sum::<f64>() / xs.len() as f64;
    var.sqrt()
}

/// The ensemble pulse per section: groove's median-IOI estimate over the
/// attack train, trusted only above the evidence gate.
fn section_tempos(
    attacks: &[Attack],
    sections: &[Section],
    config: &EnsembleConfig,
) -> Vec<SectionTempo> {
    sections
        .iter()
        .map(|s| {
            let members: Vec<f64> = attacks
                .iter()
                .map(|a| a.start_secs)
                .filter(|&t| t >= s.start_secs && t < s.end_secs)
                .collect();
            let tempo_bpm = if members.len() >= config.min_section_attacks {
                groove::analyze_groove(&members).and_then(|g| g.tempo_bpm)
            } else {
                None
            };
            SectionTempo {
                label: s.label.clone(),
                tempo_bpm,
                attack_count: members.len() as u32,
            }
        })
        .collect()
}

/// Spread across tempo'd sections, and the single largest deviation from
/// the median when it clears the accusation margin.
fn tempo_spread(
    section_tempos: &[SectionTempo],
    config: &EnsembleConfig,
) -> (Option<f32>, Option<TempoOutlier>) {
    let tempos: Vec<(usize, f64)> = section_tempos
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.tempo_bpm.map(|t| (i, t as f64)))
        .collect();
    if tempos.len() < 2 {
        return (None, None);
    }

    let values: Vec<f64> = tempos.iter().map(|&(_, t)| t).collect();
    let max = values.iter().cloned().fold(f64::MIN, f64::max);
    let min = values.iter().cloned().fold(f64::MAX, f64::min);
    let spread = (max - min) as f32;

    // Two tempo'd sections put the median at their midpoint: both deltas
    // clear any margin symmetrically and rushed-vs-dragged becomes an
    // iterator tie-break. An accusation needs three or more.
    if tempos.len() < 3 {
        return (Some(spread), None);
    }

    let median = median(&values);
    let outlier = tempos
        .iter()
        .map(|&(i, t)| (i, t - median))
        .filter(|&(_, delta)| delta.abs() > config.outlier_margin_bpm as f64)
        .filter(|&(_, delta)| !is_subdivision_ratio(median + delta, median))
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map(|(i, delta)| TempoOutlier {
            section: section_tempos[i].label.clone(),
            direction: if delta > 0.0 {
                TempoDirection::Rushed
            } else {
                TempoDirection::Dragged
            },
            delta_bpm: delta as f32,
        });

    (Some(spread), outlier)
}

/// Integer pulse multiples a section can honestly reach by changing
/// subdivision, not tempo (eighths after quarters, triplet→sixteenth
/// figures), with the relative tolerance a real band's drift stays inside.
const SUBDIVISION_RATIOS: [f64; 3] = [2.0, 3.0, 4.0];
const SUBDIVISION_RATIO_TOLERANCE: f64 = 0.12;

/// True when one pulse is a near-integer multiple of the other — a
/// subdivision change, which must never read as a tempo accusation.
fn is_subdivision_ratio(tempo: f64, reference: f64) -> bool {
    let (hi, lo) = if tempo > reference {
        (tempo, reference)
    } else {
        (reference, tempo)
    };
    if lo <= 0.0 {
        // A degenerate reference can't support an accusation either.
        return true;
    }
    let ratio = hi / lo;
    SUBDIVISION_RATIOS
        .iter()
        .any(|&k| (ratio - k).abs() <= k * SUBDIVISION_RATIO_TOLERANCE)
}

fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

/// Below this mean amplitude a half of the take is silence: it carries no
/// loudness baseline, so no direction can honestly be read from it.
const AMPLITUDE_SILENCE_FLOOR: f32 = 1e-3;

/// Band-level dynamics direction: mean amplitude of the take's first half
/// (by time) against the second.
fn balance_trend(samples: &[(f64, f32)], config: &EnsembleConfig) -> Option<BalanceTrend> {
    let mut sorted: Vec<(f64, f32)> = samples
        .iter()
        .copied()
        .filter(|(t, a)| t.is_finite() && a.is_finite())
        .collect();
    if sorted.len() < config.min_balance_samples {
        return None;
    }
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let t0 = sorted.first().map(|s| s.0).unwrap_or(0.0);
    let t1 = sorted.last().map(|s| s.0).unwrap_or(0.0);
    if t1 <= t0 {
        // Every sample on one instant: no direction to read.
        return None;
    }
    let midpoint = (t0 + t1) / 2.0;
    let (mut early_sum, mut early_n, mut late_sum, mut late_n) = (0.0f64, 0u32, 0.0f64, 0u32);
    for &(t, a) in &sorted {
        if t < midpoint {
            early_sum += a as f64;
            early_n += 1;
        } else {
            late_sum += a as f64;
            late_n += 1;
        }
    }
    if early_n == 0 || late_n == 0 {
        return None;
    }
    let early_mean = (early_sum / early_n as f64) as f32;
    let late_mean = (late_sum / late_n as f64) as f32;
    if early_mean <= AMPLITUDE_SILENCE_FLOOR {
        // A silent first half has no baseline: reporting any direction
        // next to a 0.0 mean would contradict itself on the surface.
        return None;
    }
    let change = (late_mean - early_mean) / early_mean;
    let direction = if change >= config.balance_trend_ratio {
        BalanceDirection::Rising
    } else if change <= -config.balance_trend_ratio {
        BalanceDirection::Falling
    } else {
        BalanceDirection::Steady
    };
    Some(BalanceTrend {
        direction,
        early_mean,
        late_mean,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::follower::Verdict;
    use theory::ChordQuality;

    /// A band of `offsets_secs.len()` players: each plays a steady onset
    /// train at `interval_secs`, shifted by its per-player offset. Returns
    /// the interleaved mix, unsorted on purpose (player-major order).
    fn band(offsets_secs: &[f64], interval_secs: f64, beats: usize) -> Vec<f64> {
        let mut mix = Vec::new();
        for &off in offsets_secs {
            for i in 0..beats {
                mix.push(off + i as f64 * interval_secs);
            }
        }
        mix
    }

    fn input<'a>(onsets: &'a [f64], sections: &'a [Section]) -> EnsembleInput<'a> {
        EnsembleInput {
            onsets_secs: onsets,
            amplitude_samples: &[],
            heard_chords: &[],
            chord_targets: None,
            sections,
        }
    }

    // ── AC3: the rushing player degrades togetherness and names the
    //    section (spec #349 §6 T4 AC3) ─────────────────────────────────────

    /// Two players locked 20 ms apart for 30 s vs the same band where the
    /// second player accelerates through the final third: the rushing take
    /// scores measurably less together, and the tempo outlier names "the
    /// ending" as Rushed. This is the spec's AC3 fixture.
    #[test]
    fn a_rushing_player_degrades_togetherness_and_names_the_ending() {
        let sections = default_sections(30.0);
        // Locked control: A at t, B at t+0.02, quarter = 0.6 s (100 BPM).
        let locked = band(&[0.0, 0.02], 0.6, 50);
        let locked_report =
            analyze_ensemble(&input(&locked, &sections), &EnsembleConfig::default());

        // Rushing take: A holds 100 BPM; B stays locked for two thirds,
        // then plays its final-third onsets at 0.45 s (≈133 BPM).
        let mut rushing: Vec<f64> = (0..50).map(|i| i as f64 * 0.6).collect();
        rushing.extend((0..33).map(|i| 0.02 + i as f64 * 0.6)); // B, steady to 19.22
        let mut t = 20.02;
        while t < 30.0 {
            rushing.push(t);
            t += 0.45;
        }
        let rushing_report =
            analyze_ensemble(&input(&rushing, &sections), &EnsembleConfig::default());

        let locked_t = locked_report.togetherness.expect("locked take has scatter");
        let rushing_t = rushing_report
            .togetherness
            .expect("rushing take has scatter");
        assert!(
            rushing_t.score < locked_t.score,
            "rushing must read less together: locked {} vs rushing {}",
            locked_t.score,
            rushing_t.score
        );

        let outlier = rushing_report.tempo_outlier.expect("the ending rushed");
        assert_eq!(outlier.section, "the ending");
        assert_eq!(outlier.direction, TempoDirection::Rushed);
        assert!(outlier.delta_bpm > 0.0);
        assert!(
            rushing_report.tempo_spread_bpm.expect("spread present")
                > locked_report.tempo_spread_bpm.expect("spread present"),
            "the rushing take spreads wider across sections"
        );
        // The locked control never accuses anyone — and its per-section
        // pulse is the ATTACK train (100 BPM), not the raw-onset mush.
        assert!(locked_report.tempo_outlier.is_none());
        let opening = locked_report.section_tempos[0]
            .tempo_bpm
            .expect("locked opening has a pulse");
        assert!(
            (opening - 100.0).abs() < 2.0,
            "locked pairs must read the beat, got {opening}"
        );
    }

    /// A dragging final third earns the same call in the other direction.
    #[test]
    fn a_dragging_ending_is_named_dragged() {
        let sections = default_sections(30.0);
        let mut onsets: Vec<f64> = (0..34).map(|i| i as f64 * 0.6).collect(); // to 19.8
        let mut t = 20.4;
        while t < 30.0 {
            onsets.push(t);
            t += 0.9; // ≈67 BPM against the take's 100
        }
        let report = analyze_ensemble(&input(&onsets, &sections), &EnsembleConfig::default());
        let outlier = report.tempo_outlier.expect("the ending dragged");
        assert_eq!(outlier.section, "the ending");
        assert_eq!(outlier.direction, TempoDirection::Dragged);
        assert!(outlier.delta_bpm < 0.0);
    }

    // ── Togetherness math and honesty gates ──────────────────────────────

    /// Tighter bands score higher: 10 ms offsets beat 60 ms offsets.
    #[test]
    fn togetherness_orders_tight_above_sloppy() {
        let cfg = EnsembleConfig::default();
        let sections: Vec<Section> = Vec::new();
        let tight = band(&[0.0, 0.010], 0.6, 20);
        let sloppy = band(&[0.0, 0.060], 0.6, 20);
        let tight_score = analyze_ensemble(&input(&tight, &sections), &cfg)
            .togetherness
            .expect("tight has scatter")
            .score;
        let sloppy_score = analyze_ensemble(&input(&sloppy, &sections), &cfg)
            .togetherness
            .expect("sloppy has scatter")
            .score;
        assert!(
            tight_score > sloppy_score,
            "tight {tight_score} must beat sloppy {sloppy_score}"
        );
        // 10 ms offset = 5 ms spread: at/under the tight bar → a full 1.0.
        assert!((tight_score - 1.0).abs() < 1e-6);
        // 60 ms offset = 30 ms spread: past 2/3 of the tight→loose band.
        assert!(sloppy_score < 0.5);
    }

    /// A soloist (or a band fused within onset resolution) produces only
    /// singleton clusters — togetherness stays silent instead of inventing
    /// a verdict either way.
    #[test]
    fn a_soloists_take_yields_no_togetherness_verdict() {
        let solo = band(&[0.0], 0.6, 40);
        let report = analyze_ensemble(&input(&solo, &[]), &EnsembleConfig::default());
        assert!(report.togetherness.is_none());
    }

    /// Below the scattered-cluster evidence gate: three scattered attacks
    /// are not enough to characterize a band.
    #[test]
    fn too_few_scattered_clusters_stay_silent() {
        // 3 scattered clusters (default gate is 4), padded with singletons.
        let mut onsets = band(&[0.0, 0.03], 1.0, 3);
        onsets.extend((0..10).map(|i| 20.0 + i as f64));
        let report = analyze_ensemble(&input(&onsets, &[]), &EnsembleConfig::default());
        assert!(report.togetherness.is_none());
        // One more scattered cluster clears the gate.
        let mut enough = band(&[0.0, 0.03], 1.0, 4);
        enough.extend((0..10).map(|i| 20.0 + i as f64));
        let t = analyze_ensemble(&input(&enough, &[]), &EnsembleConfig::default())
            .togetherness
            .expect("4 scattered clusters is evidence");
        assert_eq!(t.scattered_clusters, 4);
        assert_eq!(t.total_clusters, 14);
        // 30 ms offset → 15 ms population spread.
        assert!((t.mean_spread_ms - 15.0).abs() < 0.5);
    }

    /// The empty room: a report of Nones, never a panic, never a verdict.
    #[test]
    fn an_empty_take_reports_nothing() {
        let report = analyze_ensemble(&input(&[], &[]), &EnsembleConfig::default());
        assert_eq!(
            report,
            EnsembleReport {
                togetherness: None,
                section_tempos: Vec::new(),
                tempo_spread_bpm: None,
                tempo_outlier: None,
                balance: None,
                chords: None,
            }
        );
    }

    /// Unsorted input (player-major interleave) is the normal case — the
    /// analysis must sort before clustering or every cross-player gap
    /// reads as one giant cluster.
    #[test]
    fn unsorted_onsets_cluster_correctly() {
        let unsorted = band(&[0.0, 0.02], 0.6, 20); // player-major order
        let mut sorted = unsorted.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let cfg = EnsembleConfig::default();
        assert_eq!(
            analyze_ensemble(&input(&unsorted, &[]), &cfg).togetherness,
            analyze_ensemble(&input(&sorted, &[]), &cfg).togetherness
        );
    }

    /// Non-finite timestamps are dropped, not sorted: a stray NaN must not
    /// scramble the order of the real onsets around it.
    #[test]
    fn non_finite_onsets_are_dropped() {
        let clean = band(&[0.0, 0.02], 0.6, 20);
        let mut dirty = clean.clone();
        dirty.insert(7, f64::NAN);
        dirty.insert(3, f64::INFINITY);
        let cfg = EnsembleConfig::default();
        assert_eq!(
            analyze_ensemble(&input(&dirty, &[]), &cfg).togetherness,
            analyze_ensemble(&input(&clean, &[]), &cfg).togetherness
        );
    }

    // ── Sections and tempo gates ─────────────────────────────────────────

    /// A section with too few attacks gets no tempo and is excluded from
    /// the spread; below two tempo'd sections there is no spread at all.
    #[test]
    fn thin_sections_earn_no_tempo_and_no_spread() {
        let sections = default_sections(30.0);
        // Onsets only in the opening third: 100 BPM quarters.
        let onsets: Vec<f64> = (0..16).map(|i| i as f64 * 0.6).collect();
        let report = analyze_ensemble(&input(&onsets, &sections), &EnsembleConfig::default());
        assert_eq!(report.section_tempos.len(), 3);
        assert!(report.section_tempos[0].tempo_bpm.is_some());
        assert!(report.section_tempos[1].tempo_bpm.is_none());
        assert!(report.section_tempos[2].tempo_bpm.is_none());
        assert_eq!(report.section_tempos[1].attack_count, 0);
        assert!(report.tempo_spread_bpm.is_none());
        assert!(report.tempo_outlier.is_none());
    }

    /// A steady band across all sections: spread is tiny and nobody gets
    /// accused.
    #[test]
    fn a_steady_band_earns_no_accusation() {
        let sections = default_sections(30.0);
        let onsets: Vec<f64> = (0..50).map(|i| i as f64 * 0.6).collect();
        let report = analyze_ensemble(&input(&onsets, &sections), &EnsembleConfig::default());
        let spread = report.tempo_spread_bpm.expect("three tempo'd sections");
        assert!(spread < 2.0, "steady take must not spread: {spread}");
        assert!(report.tempo_outlier.is_none());
    }

    /// A real-but-small drift (~4 BPM in the ending) stays below the
    /// accusation margin: the spread reports it, nobody is named. Fails if
    /// the margin collapses toward zero.
    #[test]
    fn a_small_deviation_stays_unaccused() {
        let sections = default_sections(30.0);
        let mut onsets: Vec<f64> = (0..34).map(|i| i as f64 * 0.6).collect(); // 100 BPM to 19.8
        let mut t = 20.4;
        while t < 30.0 {
            onsets.push(t);
            t += 0.577; // ≈104 BPM
        }
        let report = analyze_ensemble(&input(&onsets, &sections), &EnsembleConfig::default());
        let spread = report.tempo_spread_bpm.expect("three tempo'd sections");
        assert!(
            (2.0..8.0).contains(&spread),
            "the drift is real and visible in the spread: {spread}"
        );
        assert!(report.tempo_outlier.is_none());
    }

    /// Switching to eighths at the SAME tempo doubles the attack density
    /// without anyone rushing: the near-integer ratio reads as a
    /// subdivision change and earns no accusation — the spread still
    /// carries the raw numbers.
    #[test]
    fn a_subdivision_change_is_not_a_rushing_accusation() {
        let sections = default_sections(30.0);
        let mut onsets: Vec<f64> = (0..34).map(|i| i as f64 * 0.6).collect(); // quarters at 100
        let mut t = 20.4;
        while t < 30.0 {
            onsets.push(t);
            t += 0.3; // eighths, same 100 BPM
        }
        let report = analyze_ensemble(&input(&onsets, &sections), &EnsembleConfig::default());
        let ending = report.section_tempos[2].tempo_bpm.expect("dense ending");
        assert!(
            (ending - 200.0).abs() < 5.0,
            "eighths read double density: {ending}"
        );
        assert!(
            report.tempo_spread_bpm.expect("spread present") > 50.0,
            "the raw spread still shows the density change"
        );
        assert!(
            report.tempo_outlier.is_none(),
            "a subdivision change must never be called rushing: {:?}",
            report.tempo_outlier
        );
    }

    /// With exactly two tempo'd sections the median sits at their
    /// midpoint, so rushed-vs-dragged would be an iterator tie-break:
    /// no accusation below three, while the spread still reports.
    #[test]
    fn two_tempod_sections_never_earn_an_accusation() {
        let sections = vec![
            Section {
                label: "the head".to_string(),
                start_secs: 0.0,
                end_secs: 10.0,
            },
            Section {
                label: "the solos".to_string(),
                start_secs: 10.0,
                end_secs: 20.0,
            },
        ];
        let mut onsets: Vec<f64> = (0..17).map(|i| i as f64 * 0.6).collect(); // 100 BPM
        onsets.extend((0..20).map(|i| 10.2 + i as f64 * 0.5)); // 120 BPM
        let report = analyze_ensemble(&input(&onsets, &sections), &EnsembleConfig::default());
        let spread = report.tempo_spread_bpm.expect("both sections tempo'd");
        assert!(
            (spread - 20.0).abs() < 2.0,
            "spread reports the gap: {spread}"
        );
        assert!(report.tempo_outlier.is_none());
    }

    /// A section holding 2–7 attacks is below the evidence gate: no tempo,
    /// even though groove could compute one from two onsets. Fails if the
    /// gate is deleted or lowered to groove's own ≥2 minimum.
    #[test]
    fn a_thin_middle_earns_no_tempo() {
        let sections = default_sections(30.0);
        let mut onsets: Vec<f64> = (0..17).map(|i| i as f64 * 0.6).collect(); // opening
        onsets.extend((0..5).map(|i| 10.2 + i as f64 * 0.6)); // 5 attacks
        onsets.extend((0..16).map(|i| 20.4 + i as f64 * 0.6)); // ending
        let report = analyze_ensemble(&input(&onsets, &sections), &EnsembleConfig::default());
        assert_eq!(report.section_tempos[1].attack_count, 5);
        assert!(report.section_tempos[1].tempo_bpm.is_none());
        assert!(report.section_tempos[0].tempo_bpm.is_some());
        assert!(report.section_tempos[2].tempo_bpm.is_some());
    }

    /// default_sections covers the take in labeled thirds and the final
    /// onset is not dropped by the boundary; degenerate durations yield
    /// nothing.
    #[test]
    fn default_sections_cover_the_take() {
        let s = default_sections(30.0);
        assert_eq!(s.len(), 3);
        assert_eq!(
            s.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(),
            vec!["the opening", "the middle", "the ending"]
        );
        assert_eq!(s[0].start_secs, 0.0);
        assert!((s[0].end_secs - 10.0).abs() < 1e-9);
        assert!((s[2].start_secs - 20.0).abs() < 1e-9);
        // An onset exactly at the take's end still lands in the ending.
        let onsets: Vec<f64> = (0..51).map(|i| i as f64 * 0.6).collect(); // last = 30.0
        let report = analyze_ensemble(&input(&onsets, &s), &EnsembleConfig::default());
        assert_eq!(report.section_tempos[2].attack_count, 17);
        assert!(default_sections(0.0).is_empty());
        assert!(default_sections(-5.0).is_empty());
    }

    // ── Balance trend ────────────────────────────────────────────────────

    fn amp_ramp(from: f32, to: f32, n: usize) -> Vec<(f64, f32)> {
        (0..n)
            .map(|i| {
                let frac = i as f32 / (n - 1) as f32;
                (i as f64, from + (to - from) * frac)
            })
            .collect()
    }

    fn amp_input(amps: &[(f64, f32)]) -> EnsembleInput<'_> {
        EnsembleInput {
            onsets_secs: &[],
            amplitude_samples: amps,
            heard_chords: &[],
            chord_targets: None,
            sections: &[],
        }
    }

    /// A crescendo reads Rising, a decrescendo Falling, a flat take
    /// Steady — and each carries the halves' means.
    #[test]
    fn balance_trend_reads_direction_honestly() {
        let cfg = EnsembleConfig::default();
        let rising = amp_ramp(0.2, 0.8, 20);
        let b = analyze_ensemble(&amp_input(&rising), &cfg).balance.unwrap();
        assert_eq!(b.direction, BalanceDirection::Rising);
        assert!(b.late_mean > b.early_mean);

        let falling = amp_ramp(0.8, 0.2, 20);
        let b = analyze_ensemble(&amp_input(&falling), &cfg)
            .balance
            .unwrap();
        assert_eq!(b.direction, BalanceDirection::Falling);

        let steady = amp_ramp(0.5, 0.52, 20);
        let b = analyze_ensemble(&amp_input(&steady), &cfg).balance.unwrap();
        assert_eq!(b.direction, BalanceDirection::Steady);
    }

    /// Too few samples, or a take with no time span, stays silent — and a
    /// silent first half is an evidence failure, not a `Steady` verdict: a
    /// surface rendering "steady" next to means of 0.0 and 0.6 would
    /// contradict itself.
    #[test]
    fn balance_gates_on_evidence() {
        let cfg = EnsembleConfig::default();
        let few = amp_ramp(0.2, 0.8, 7); // default gate is 8
        assert!(analyze_ensemble(&amp_input(&few), &cfg).balance.is_none());

        let instant: Vec<(f64, f32)> = (0..10).map(|_| (1.0, 0.5)).collect();
        assert!(analyze_ensemble(&amp_input(&instant), &cfg)
            .balance
            .is_none());

        let silent_start: Vec<(f64, f32)> = (0..20)
            .map(|i| (i as f64, if i < 10 { 0.0 } else { 0.6 }))
            .collect();
        assert!(analyze_ensemble(&amp_input(&silent_start), &cfg)
            .balance
            .is_none());
    }

    /// The direction margin is real: a +12% step reads Steady under the
    /// default ±15% ratio. Fails if the margin collapses toward zero.
    #[test]
    fn a_small_loudness_step_reads_steady() {
        let cfg = EnsembleConfig::default();
        let step12: Vec<(f64, f32)> = (0..20)
            .map(|i| (i as f64, if i < 10 { 0.50 } else { 0.56 }))
            .collect();
        let b = analyze_ensemble(&amp_input(&step12), &cfg).balance.unwrap();
        assert_eq!(b.direction, BalanceDirection::Steady);
    }

    // ── Group chord verdict (T2 judging over the mix) ────────────────────

    /// The band's mix judged as one player: a hit, a near (right root,
    /// wrong quality), and an unplayed cell closing Missed — accuracy
    /// divides by all targets. `chords` stays `None` without material.
    #[test]
    fn the_mix_is_judged_as_one_player() {
        let targets = vec![
            ChordTarget {
                segment: 0,
                root_pc: 0,
                quality: ChordQuality::Dom7,
                bass_pc: None,
            },
            ChordTarget {
                segment: 1,
                root_pc: 5,
                quality: ChordQuality::Dom7,
                bass_pc: None,
            },
            ChordTarget {
                segment: 2,
                root_pc: 10,
                quality: ChordQuality::Dom7,
                bass_pc: None,
            },
        ];
        let heard = vec![
            HeardChord {
                root_pc: 0,
                quality: ChordQuality::Dom7,
                bass_pc: None,
            },
            HeardChord {
                root_pc: 5,
                quality: ChordQuality::Maj,
                bass_pc: None,
            },
        ];
        let onsets = band(&[0.0, 0.02], 0.6, 10);
        let report = analyze_ensemble(
            &EnsembleInput {
                onsets_secs: &onsets,
                amplitude_samples: &[],
                heard_chords: &heard,
                chord_targets: Some(&targets),
                sections: &[],
            },
            &EnsembleConfig::default(),
        );
        let chords = report.chords.expect("material was loaded");
        assert_eq!(chords.verdicts.len(), 3);
        assert_eq!(chords.verdicts[0].verdict, Verdict::Hit);
        assert_eq!(chords.verdicts[1].verdict, Verdict::Near);
        assert_eq!(chords.verdicts[2].verdict, Verdict::Missed);
        assert!((chords.accuracy - 0.5).abs() < 1e-6);

        // No material loaded → no chord verdict, even with chords heard.
        let no_material = analyze_ensemble(
            &EnsembleInput {
                onsets_secs: &onsets,
                amplitude_samples: &[],
                heard_chords: &heard,
                chord_targets: None,
                sections: &[],
            },
            &EnsembleConfig::default(),
        );
        assert!(no_material.chords.is_none());
    }

    /// Material loaded but the room never matched it: every cell closes an
    /// honest Missed at zero accuracy — never a skipped judgment.
    #[test]
    fn unplayed_material_grades_zero_not_absent() {
        let targets = vec![ChordTarget {
            segment: 0,
            root_pc: 2,
            quality: ChordQuality::Min7,
            bass_pc: None,
        }];
        let verdict = judge_group_chords(&targets, &[]);
        assert_eq!(verdict.verdicts.len(), 1);
        assert_eq!(verdict.verdicts[0].verdict, Verdict::Missed);
        assert_eq!(verdict.accuracy, 0.0);
    }

    // ── Contract plumbing ────────────────────────────────────────────────

    /// The report round-trips through serde losslessly — the recap and
    /// IPC layers will carry it as JSON.
    #[test]
    fn ensemble_report_serde_roundtrip() {
        let sections = default_sections(30.0);
        let onsets = band(&[0.0, 0.025], 0.6, 50);
        let amps = amp_ramp(0.3, 0.7, 20);
        let report = analyze_ensemble(
            &EnsembleInput {
                onsets_secs: &onsets,
                amplitude_samples: &amps,
                heard_chords: &[],
                chord_targets: None,
                sections: &sections,
            },
            &EnsembleConfig::default(),
        );
        assert!(report.togetherness.is_some());
        assert!(report.balance.is_some());
        let json = serde_json::to_string(&report).expect("serialize");
        let back: EnsembleReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, back);
    }
}
