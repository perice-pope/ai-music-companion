//! Reveal connections — "what real-world music lives in what you just played".
//!
//! While the player free-plays, the live `perception` reading (key + mode +
//! confidence) is turned into an occasional **reveal**: a real artist or piece
//! that lives in that scale/mode, plus a one-line why. This is the ambient,
//! collectible delight of the practice coach (epic #252, feature #253).
//!
//! Slice 1 is deliberately small and self-contained:
//! - **Grounded only.** Every connection comes from the curated table below, so
//!   the app never confidently invents a wrong "that's Coltrane" (it's a kids'
//!   tool). The LLM enrichment of the `why` line is a later slice and stays
//!   opt-in + disclosed.
//! - **Offline + pure.** Selection is a pure function of `(context, seed)` with
//!   no I/O, so it's fully deterministic and unit-testable, and it makes no
//!   network call at all.
//! - **Calm.** A reveal surfaces at most once every `DEFAULT_REVEAL_CADENCE`
//!   completed phrases.

use serde::{Deserialize, Serialize};

/// Minimum perception confidence before we offer a reveal. Below this we stay
/// silent rather than guess.
pub const REVEAL_MIN_CONFIDENCE: f32 = 0.6;

/// How often a reveal may surface, in completed phrases (at most one per N).
pub const DEFAULT_REVEAL_CADENCE: usize = 3;

/// Provenance of a reveal's wording. In slice 1 every reveal is [`Grounded`]
/// (verbatim from the curated table, no network). A later slice adds
/// [`LlmGrounded`] (the LLM may reword the `why`, but the artist/piece still
/// comes from the table).
///
/// [`Grounded`]: RevealSource::Grounded
/// [`LlmGrounded`]: RevealSource::LlmGrounded
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealSource {
    /// Straight from the curated table — no network, no model.
    Grounded,
    /// Wording enriched by the LLM, but still grounded in a curated connection.
    LlmGrounded,
}

/// A real-world music connection for what the player is doing right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reveal {
    /// The musical concept detected, e.g. `"G Dorian"`.
    pub concept: String,
    /// The grounded real-world connection, e.g. `"Miles Davis — \"So What\""`.
    pub connection: String,
    /// One line on why (kept short, &le; ~140 chars).
    pub why: String,
    /// Where the wording came from.
    pub source: RevealSource,
}

/// What the app currently hears, distilled to what a reveal needs. Built from
/// the existing `perception` snapshot's `KeySnapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicalContext {
    /// Tonic pitch class, 0&ndash;11 (C = 0).
    pub tonic: u8,
    /// Mode label as perception reports it, e.g. `"major"` / `"minor"` /
    /// `"Dorian"`. Matched case-insensitively.
    pub mode: String,
    /// Best-fit confidence, 0&ndash;1.
    pub confidence: f32,
}

/// One curated exemplar: a grounded connection plus its one-line "why".
struct Exemplar {
    connection: &'static str,
    why: &'static str,
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Curated, grounded exemplars per mode, keyed by a normalized (lowercased) mode
/// label. The artist/piece is factually associated with that mode — that
/// grounding is what keeps reveals honest. Returns the display label for the
/// mode and its exemplars. Expand this table over time; an unknown mode yields
/// no reveal (we never fabricate).
fn curated_for(mode_normalized: &str) -> Option<(&'static str, &'static [Exemplar])> {
    match mode_normalized {
        "major" | "ionian" => Some((
            "Major",
            &[
                Exemplar {
                    connection: "Beethoven — \"Ode to Joy\"",
                    why: "The bright, settled 'home' sound behind most anthems and nursery tunes.",
                },
                Exemplar {
                    connection: "most pop & folk songs",
                    why: "The major scale is the default happy, resolved sound of Western music.",
                },
            ],
        )),
        "minor" | "aeolian" => Some((
            "Minor",
            &[
                Exemplar {
                    connection: "Beethoven — \"Moonlight Sonata\"",
                    why: "The darker, more serious natural-minor color.",
                },
                Exemplar {
                    connection: "much film & game 'tension' music",
                    why: "Natural minor is the go-to for sadness, mystery, and weight.",
                },
            ],
        )),
        "dorian" => Some((
            "Dorian",
            &[
                Exemplar {
                    connection: "Miles Davis — \"So What\"",
                    why: "A cool, jazzy minor with a bright raised 6th — the sound of modal jazz.",
                },
                Exemplar {
                    connection: "Santana — \"Oye Como Va\"",
                    why: "That smooth Latin-rock groove rides on Dorian.",
                },
            ],
        )),
        "phrygian" => Some((
            "Phrygian",
            &[
                Exemplar {
                    connection: "Flamenco music",
                    why: "The Spanish sound comes from Phrygian's dramatic flat-2nd.",
                },
                Exemplar {
                    connection: "Metallica — \"Wherever I May Roam\"",
                    why: "Metal loves Phrygian's dark, exotic edge.",
                },
            ],
        )),
        "lydian" => Some((
            "Lydian",
            &[
                Exemplar {
                    connection: "\"The Simpsons\" theme",
                    why: "That floating, dreamy 'wonder' sound is Lydian's raised 4th.",
                },
                Exemplar {
                    connection: "film scores of awe & space",
                    why: "Composers reach for Lydian to sound magical and wide-eyed.",
                },
            ],
        )),
        "mixolydian" => Some((
            "Mixolydian",
            &[
                Exemplar {
                    connection: "Lynyrd Skynyrd — \"Sweet Home Alabama\"",
                    why: "A bluesy major with a flat-7th — the backbone of a lot of rock.",
                },
                Exemplar {
                    connection: "Celtic & folk tunes",
                    why: "Mixolydian's flat-7th gives jigs and reels their old-world lilt.",
                },
            ],
        )),
        "locrian" => Some((
            "Locrian",
            &[Exemplar {
                connection: "Björk — \"Army of Me\"",
                why: "Rare and unstable — Locrian's flat-5th never quite rests.",
            }],
        )),
        "harmonic minor" => Some((
            "Harmonic Minor",
            &[Exemplar {
                connection: "Classical & Middle-Eastern music",
                why: "The raised 7th gives that exotic, dramatic minor leap.",
            }],
        )),
        "melodic minor" => Some((
            "Melodic Minor",
            &[Exemplar {
                connection: "jazz improvisation",
                why: "Raising the 6th and 7th smooths the climb — a jazz staple.",
            }],
        )),
        "major pentatonic" => Some((
            "Major Pentatonic",
            &[Exemplar {
                connection: "\"Amazing Grace\" & much folk",
                why: "Five notes that can't clash — the friendliest scale there is.",
            }],
        )),
        "minor pentatonic" | "blues" => Some((
            "Minor Pentatonic",
            &[Exemplar {
                connection: "B.B. King & blues-rock solos",
                why: "The soul of blues and rock lead playing.",
            }],
        )),
        _ => None,
    }
}

/// Pure, deterministic reveal selection.
///
/// Returns `None` below [`REVEAL_MIN_CONFIDENCE`] or when the mode has no
/// curated match (it never fabricates). For a given `(ctx, seed)` it always
/// returns the same exemplar, so it's trivially testable.
pub fn reveal_for(ctx: &MusicalContext, seed: u64) -> Option<Reveal> {
    if ctx.confidence < REVEAL_MIN_CONFIDENCE {
        return None;
    }
    let mode_key = ctx.mode.trim().to_lowercase();
    let (display_mode, exemplars) = curated_for(&mode_key)?;
    if exemplars.is_empty() {
        return None;
    }
    let note = NOTE_NAMES[(ctx.tonic % 12) as usize];
    let pick = &exemplars[(seed as usize) % exemplars.len()];
    Some(Reveal {
        concept: format!("{note} {display_mode}"),
        connection: pick.connection.to_string(),
        why: pick.why.to_string(),
        source: RevealSource::Grounded,
    })
}

/// Cadence-limited reveal for a just-completed phrase.
///
/// Surfaces at most one reveal every `cadence` phrases (so the coach never
/// spams) and only when [`reveal_for`] has a confident, grounded match. The
/// cadence is a pure function of `phrase_index`, so no engine state is needed
/// and the whole thing stays deterministic.
pub fn reveal_on_phrase(
    ctx: &MusicalContext,
    phrase_index: usize,
    cadence: usize,
) -> Option<Reveal> {
    if cadence == 0 || !(phrase_index + 1).is_multiple_of(cadence) {
        return None;
    }
    reveal_for(ctx, phrase_index as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(tonic: u8, mode: &str, confidence: f32) -> MusicalContext {
        MusicalContext {
            tonic,
            mode: mode.to_string(),
            confidence,
        }
    }

    /// AC1: a confident, curated context yields a reveal whose connection is one
    /// of the curated exemplars for that mode (and the concept names the key).
    /// Fails if selection ever returns an off-table connection.
    #[test]
    fn returns_curated_exemplar() {
        let r = reveal_for(&ctx(7, "Dorian", 0.9), 0).expect("confident Dorian should reveal");
        assert_eq!(r.concept, "G Dorian");
        assert!(
            r.connection.contains("So What") || r.connection.contains("Oye Como Va"),
            "connection must be a curated Dorian exemplar, got {:?}",
            r.connection
        );
        assert_eq!(r.source, RevealSource::Grounded);
    }

    /// AC2: below the confidence threshold we stay silent rather than guess.
    /// Fails if a low-confidence reading ever produces a reveal.
    #[test]
    fn low_confidence_returns_none() {
        assert!(reveal_for(&ctx(7, "Dorian", REVEAL_MIN_CONFIDENCE - 0.01), 0).is_none());
    }

    /// AC3: an unknown/unmapped mode yields nothing — never a fabricated match.
    /// Fails if the table ever falls through to a default connection.
    #[test]
    fn no_match_returns_none() {
        assert!(reveal_for(&ctx(0, "Bebop Super Locrian", 0.99), 0).is_none());
    }

    /// AC6: selection is deterministic for a fixed `(ctx, seed)`, and the seed
    /// actually rotates exemplars (Dorian has two). Fails if selection became
    /// non-deterministic or stopped using the seed.
    #[test]
    fn reveal_is_deterministic_and_seed_rotates() {
        let c = ctx(7, "dorian", 0.9);
        assert_eq!(reveal_for(&c, 0), reveal_for(&c, 0));
        let a = reveal_for(&c, 0).unwrap().connection;
        let b = reveal_for(&c, 1).unwrap().connection;
        assert_ne!(
            a, b,
            "different seeds should pick the two different Dorian exemplars"
        );
    }

    /// AC5: slice-1 reveals are produced with no network/LLM in scope — the
    /// source is always `Grounded`. Fails if an LLM/network path is ever wired
    /// into S1 (the source would become `LlmGrounded`), flagging the scope creep.
    #[test]
    fn slice1_reveals_are_grounded_offline() {
        let r = reveal_for(&ctx(0, "major", 0.9), 0).unwrap();
        assert_eq!(r.source, RevealSource::Grounded);
    }

    /// AC4: at most one reveal per `cadence` phrases. Over 5 rapid, confident,
    /// curated phrases with cadence 3, no more than 2 reveals surface (here,
    /// exactly one, on the 3rd). Fails if cadence gating regresses to per-phrase.
    #[test]
    fn rate_limited_per_n_phrases() {
        let c = ctx(7, "dorian", 0.9);
        let count = (0..5)
            .filter(|&i| reveal_on_phrase(&c, i, 3).is_some())
            .count();
        assert!(
            count <= 2,
            "expected <=2 reveals over 5 phrases, got {count}"
        );
        assert_eq!(
            count, 1,
            "with cadence 3 over 5 phrases exactly one should fire"
        );
        // And the one that fires is on the 3rd phrase (index 2).
        assert!(reveal_on_phrase(&c, 2, 3).is_some());
        assert!(reveal_on_phrase(&c, 0, 3).is_none());
    }

    /// A cadence of 0 is treated as "never" rather than dividing by zero.
    #[test]
    fn zero_cadence_never_reveals() {
        assert!(reveal_on_phrase(&ctx(7, "dorian", 0.9), 2, 0).is_none());
    }
}
