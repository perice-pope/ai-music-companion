//! "Your sound" mirror (#258): the F2 `derive_sound_profile` transition —
//! a pure, deterministic aggregate of stored session fingerprints into the
//! player's evolving identity. Absent axes mean "not measured honestly yet"
//! (the fingerprint contract), never a zeroed guess; the real-world comparison
//! is grounded in #253's curated table only — never fabricated.

use serde::{Deserialize, Serialize};

use crate::connections::RevealSource;
use crate::fingerprint::MusicalFingerprint;
use crate::store::TasteProfile;

/// Below this many stored fingerprints the mirror stays dark ("keep playing").
pub const MIN_SESSIONS: usize = 5;
/// The comparison line only shows at/above this overall confidence.
pub const COMPARISON_MIN_CONFIDENCE: f32 = 0.5;

/// Minor- vs major-family tendency across sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeLean {
    Minor,
    Major,
    Balanced,
}

/// Swing tendency across sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feel {
    Swung,
    Straight,
    Mixed,
}

/// The grounded real-world comparison ("shades of …").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub label: String,
    pub source: RevealSource,
}

/// The player's derived sound identity — a snapshot stored on the Learner
/// Model blob (forward-compatible: every field defaults, so partial blobs
/// from other versions still parse).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SoundProfile {
    #[serde(default)]
    pub sessions_counted: u32,
    #[serde(default)]
    pub mode_lean: Option<ModeLean>,
    #[serde(default)]
    pub feel: Option<Feel>,
    #[serde(default)]
    pub comparison: Option<Comparison>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub derived_at: i64,
}

/// A family lean needs at least this fraction on one side.
const LEAN_FRACTION: f32 = 0.6;
/// Swing-ratio bands (long/short IOI ratio: 1.0 = straight, 1.5 = triplet).
const SWUNG_MIN: f32 = 1.2;
const STRAIGHT_MAX: f32 = 1.08;

fn is_minor_family(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "minor" | "aeolian" | "dorian" | "phrygian" | "locrian" | "harmonic minor"
    )
}

/// Aggregate ≥[`MIN_SESSIONS`] fingerprints (oldest → newest) into the
/// player's sound identity. Pure: same inputs ⇒ same profile; `now` only
/// stamps `derived_at`, never classifies.
pub fn derive_sound_profile(
    sessions: &[MusicalFingerprint],
    taste: &TasteProfile,
    now: i64,
) -> Option<SoundProfile> {
    if sessions.len() < MIN_SESSIONS {
        return None;
    }

    // --- mode lean: minor- vs major-family over sessions with a key ---------
    let mode_labels: Vec<&'static str> = sessions
        .iter()
        .filter_map(|s| s.key.as_ref().map(|k| k.mode.label()))
        .collect();
    let (mode_lean, mode_decisiveness) = if mode_labels.is_empty() {
        (None, None)
    } else {
        let minor = mode_labels.iter().filter(|l| is_minor_family(l)).count() as f32;
        let frac = minor / mode_labels.len() as f32;
        let lean = if frac >= LEAN_FRACTION {
            ModeLean::Minor
        } else if frac <= 1.0 - LEAN_FRACTION {
            ModeLean::Major
        } else {
            ModeLean::Balanced
        };
        // How far from the fence the vote sits, 0..1.
        (Some(lean), Some((frac - 0.5).abs() * 2.0))
    };

    // --- feel: median swing ratio over sessions that reported one -----------
    let mut ratios: Vec<f32> = sessions
        .iter()
        .filter_map(|s| s.groove.as_ref().and_then(|g| g.swing_ratio))
        .collect();
    let (feel, feel_decisiveness) = if ratios.is_empty() {
        (None, None)
    } else {
        ratios.sort_by(|a, b| a.total_cmp(b));
        let median = ratios[ratios.len() / 2];
        let class_of = |r: f32| {
            if r >= SWUNG_MIN {
                Feel::Swung
            } else if r <= STRAIGHT_MAX {
                Feel::Straight
            } else {
                Feel::Mixed
            }
        };
        let feel = class_of(median);
        // Agreement: fraction of sessions matching the median's class.
        let agree =
            ratios.iter().filter(|&&r| class_of(r) == feel).count() as f32 / ratios.len() as f32;
        (Some(feel), Some(agree))
    };

    // --- confidence: sessions seen × axis agreement --------------------------
    let decisiveness: Vec<f32> = [mode_decisiveness, feel_decisiveness]
        .into_iter()
        .flatten()
        .collect();
    let confidence = if decisiveness.is_empty() {
        0.0
    } else {
        let agreement = decisiveness.iter().sum::<f32>() / decisiveness.len() as f32;
        let volume = (sessions.len() as f32 / 10.0).min(1.0);
        (agreement * volume).clamp(0.0, 1.0)
    };

    // --- comparison: grounded only, taste-preferred --------------------------
    let comparison = if confidence >= COMPARISON_MIN_CONFIDENCE {
        // The dominant concrete mode label carries the lookup (family leans
        // are too coarse for the curated table).
        dominant(&mode_labels).and_then(|label| {
            crate::connections::curated_for(&label.to_lowercase()).map(|(_, exemplars)| {
                let wants: Vec<String> = taste
                    .artists
                    .iter()
                    .chain(taste.genres.iter())
                    .map(|s| s.to_lowercase())
                    .collect();
                let pick = exemplars
                    .iter()
                    .find(|e| {
                        let c = e.connection.to_lowercase();
                        wants.iter().any(|w| c.contains(w.as_str()))
                    })
                    .or_else(|| exemplars.first())
                    .expect("curated rows are non-empty");
                Comparison {
                    label: format!("shades of {}", pick.connection),
                    source: RevealSource::Grounded,
                }
            })
        })
    } else {
        None
    };

    Some(SoundProfile {
        sessions_counted: sessions.len() as u32,
        mode_lean,
        feel,
        comparison,
        confidence,
        derived_at: now,
    })
}

/// Most frequent element (ties broken by first occurrence — deterministic).
fn dominant<'a>(labels: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for &l in labels {
        let count = labels.iter().filter(|&&x| x == l).count();
        match best {
            Some((_, c)) if count <= c => {}
            _ => best = Some((l, count)),
        }
    }
    best.map(|(l, _)| l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use theory::{KeyEstimate, Mode};

    fn fp(mode: Option<Mode>, swing: Option<f32>) -> MusicalFingerprint {
        MusicalFingerprint {
            tone: None,
            key: mode.map(|m| KeyEstimate {
                tonic: 2,
                mode: m,
                confidence: 0.8,
                margin: 0.2,
            }),
            intonation: None,
            groove: swing.map(|s| groove::GrooveDescriptor {
                tempo_bpm: Some(100.0),
                swing_ratio: Some(s),
                timing_consistency: 0.8,
                mean_ioi_secs: 0.3,
                onset_count: 24,
            }),
        }
    }

    fn taste(artists: &[&str]) -> TasteProfile {
        TasteProfile {
            artists: artists.iter().map(|s| (*s).to_owned()).collect(),
            ..TasteProfile::default()
        }
    }

    /// #258 AC (empty state): below MIN_SESSIONS the mirror stays dark — no
    /// guessed identity from two sessions.
    #[test]
    fn below_min_sessions_is_none() {
        let sessions: Vec<_> = (0..MIN_SESSIONS - 1)
            .map(|_| fp(Some(Mode::Dorian), Some(1.5)))
            .collect();
        assert!(derive_sound_profile(&sessions, &TasteProfile::default(), 1).is_none());
    }

    /// #258 AC: a consistently dark-and-swung player reads Minor + Swung with
    /// a grounded comparison; determinism pinned by running it twice.
    #[test]
    fn consistent_sessions_resolve_the_axes() {
        let sessions: Vec<_> = (0..6).map(|_| fp(Some(Mode::Dorian), Some(1.4))).collect();
        let p = derive_sound_profile(&sessions, &TasteProfile::default(), 7).unwrap();
        assert_eq!(p.mode_lean, Some(ModeLean::Minor));
        assert_eq!(p.feel, Some(Feel::Swung));
        assert!(p.confidence >= COMPARISON_MIN_CONFIDENCE);
        let c = p.comparison.as_ref().expect("confident → comparison");
        assert_eq!(c.source, RevealSource::Grounded);
        assert!(c.label.starts_with("shades of "));
        assert_eq!(
            derive_sound_profile(&sessions, &TasteProfile::default(), 7).unwrap(),
            p,
            "pure: same inputs, same profile"
        );
    }

    /// #258 AC (taste grounding): when the player's stated artists intersect a
    /// curated exemplar, THAT exemplar wins the comparison (Dorian carries
    /// both Miles Davis and Santana rows).
    #[test]
    fn comparison_prefers_the_players_taste() {
        let sessions: Vec<_> = (0..6).map(|_| fp(Some(Mode::Dorian), Some(1.4))).collect();
        let p = derive_sound_profile(&sessions, &taste(&["Santana"]), 7).unwrap();
        assert!(
            p.comparison.unwrap().label.contains("Santana"),
            "taste intersection must steer the exemplar"
        );
    }

    /// #258 AC (honest absence): sessions with no key and no groove resolve NO
    /// axes, zero confidence, and no comparison — never a fabricated identity.
    #[test]
    fn unmeasured_axes_stay_absent() {
        let sessions: Vec<_> = (0..6).map(|_| fp(None, None)).collect();
        let p = derive_sound_profile(&sessions, &TasteProfile::default(), 7).unwrap();
        assert_eq!(p.mode_lean, None);
        assert_eq!(p.feel, None);
        assert_eq!(p.confidence, 0.0);
        assert!(p.comparison.is_none(), "no evidence → no comparison, ever");
    }

    /// Mixed evidence lowers confidence below the comparison bar: half major /
    /// half minor sessions read Balanced with the comparison withheld.
    #[test]
    fn wobbly_evidence_withholds_the_comparison() {
        let mut sessions: Vec<_> = (0..3).map(|_| fp(Some(Mode::Ionian), Some(1.0))).collect();
        sessions.extend((0..3).map(|_| fp(Some(Mode::Aeolian), Some(1.5))));
        let p = derive_sound_profile(&sessions, &TasteProfile::default(), 7).unwrap();
        assert_eq!(p.mode_lean, Some(ModeLean::Balanced));
        assert!(p.confidence < COMPARISON_MIN_CONFIDENCE);
        assert!(p.comparison.is_none());
    }
}
