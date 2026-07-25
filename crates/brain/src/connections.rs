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
///
/// Calibrated to the note-gated key signal (#321/#325): steady one-key
/// melodic material correlates ≥ ~0.67 (five equal-length scale tones — the
/// worst honest case) while semi-chromatic noodling tops out ~0.57, so 0.6
/// splits them. #266 had raised the gate to 0.72 against the old per-FRAME
/// signal, whose spikes it was damping — on the calm per-note signal that
/// value sits above everything but a full tonic-emphasized scale and muted
/// reveals entirely (#353). Wrong-key protection does not ride this gate:
/// the card carries its generating key and the UI dismisses it when the
/// live reading moves off it (#266's structural fix, unchanged).
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
    /// The musical concept the card may claim, e.g. `"Dorian"`. This is the
    /// **mode only** — the curated exemplars are mode-level, so stamping the
    /// heard key onto them ("G# Major — Ode to Joy") fabricated a key claim
    /// the catalog never made (#388). The heard key still rides along as
    /// `tonic`/`mode` for dismissal and practice, it just isn't asserted.
    pub concept: String,
    /// The grounded real-world connection, e.g. `"Miles Davis — \"So What\""`.
    pub connection: String,
    /// One line on why (kept short, &le; ~140 chars).
    pub why: String,
    /// Where the wording came from.
    pub source: RevealSource,
    /// The tonic pitch class (0&ndash;11) this reveal was generated for. The UI
    /// compares it against the live perception key and dismisses the card once
    /// the detected key moves off it, so a lingering card can't contradict the
    /// "I hear" header (#266).
    pub tonic: u8,
    /// The normalized (lowercased) mode this reveal was generated for, e.g.
    /// `"dorian"`. Paired with `tonic` for the same live-key comparison.
    pub mode: String,
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
pub(crate) struct Exemplar {
    pub(crate) connection: &'static str,
    pub(crate) why: &'static str,
}

/// Curated, grounded exemplars per mode, keyed by a normalized (lowercased) mode
/// label. The artist/piece is factually associated with that mode — that
/// grounding is what keeps reveals honest. Returns the display label for the
/// mode and its exemplars. Expand this table over time; an unknown mode yields
/// no reveal (we never fabricate).
pub(crate) fn curated_for(mode_normalized: &str) -> Option<(&'static str, &'static [Exemplar])> {
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
        // Forward-looking: perception (`theory::Mode`) currently only emits the
        // seven church modes (+ major/minor), so the arms below don't fire on
        // the live path yet. They're kept ready for a richer scale detector and
        // exercised directly by `reveal_for` unit tests.
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
    // Stay silent below the confidence threshold — and treat a NaN reading as
    // "not confident" rather than letting `NaN < threshold == false` slip
    // through the gate.
    if ctx.confidence.is_nan() || ctx.confidence < REVEAL_MIN_CONFIDENCE {
        return None;
    }
    let mode_key = ctx.mode.trim().to_lowercase();
    let (display_mode, exemplars) = curated_for(&mode_key)?;
    if exemplars.is_empty() {
        return None;
    }
    let tonic = ctx.tonic % 12;
    let pick = &exemplars[(seed as usize) % exemplars.len()];
    // #388: attribute by MODE, never by the heard key. The exemplars are
    // genuinely associated with the mode; naming the key here ("G# Major —
    // Ode to Joy") claimed the piece lives in a key it doesn't.
    Some(Reveal {
        concept: display_mode.to_string(),
        connection: pick.connection.to_string(),
        why: pick.why.to_string(),
        source: RevealSource::Grounded,
        tonic,
        mode: mode_key,
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

/// Fold an optional LLM-enriched `why` into a grounded reveal (#253 S2). A
/// non-empty rewrite replaces `why` and marks the source [`RevealSource::LlmGrounded`];
/// anything else (`None` / blank) keeps the curated reveal unchanged. `concept`
/// and `connection` are **never** touched — the model only ever rewords `why`,
/// so a reveal can't drift off its grounded artist/piece.
pub fn apply_enriched_why(reveal: Reveal, enriched_why: Option<String>) -> Reveal {
    match enriched_why {
        Some(why) if !why.trim().is_empty() => Reveal {
            why,
            source: RevealSource::LlmGrounded,
            ..reveal
        },
        _ => reveal,
    }
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
    /// of the curated exemplars for that mode (and the concept names the mode).
    /// Fails if selection ever returns an off-table connection.
    #[test]
    fn returns_curated_exemplar() {
        let r = reveal_for(&ctx(7, "Dorian", 0.9), 0).expect("confident Dorian should reveal");
        assert_eq!(r.concept, "Dorian");
        assert!(
            r.connection.contains("So What") || r.connection.contains("Oye Como Va"),
            "connection must be a curated Dorian exemplar, got {:?}",
            r.connection
        );
        assert_eq!(r.source, RevealSource::Grounded);
    }

    /// #388: the concept attributes the exemplar by MODE, never by the heard
    /// key — the curated songs are mode-level, so a key-stamped headline
    /// ("G# Major — Ode to Joy") is fabrication. Pinned by asserting the
    /// concept is byte-identical across all 12 tonics (a key claim would make
    /// them differ) and exactly the mode's display label.
    #[test]
    fn concept_attributes_mode_never_the_heard_key() {
        for (mode, display) in [
            ("major", "Major"),
            ("dorian", "Dorian"),
            ("locrian", "Locrian"),
        ] {
            for tonic in 0..12u8 {
                let r = reveal_for(&ctx(tonic, mode, 0.9), 0)
                    .expect("confident curated mode should reveal");
                assert_eq!(
                    r.concept, display,
                    "concept for tonic {tonic} must claim only the mode"
                );
                assert_eq!(r.tonic, tonic, "the heard key still rides along as data");
            }
        }
    }

    /// #266 AC1: the reveal carries the key it was generated for — `tonic` and
    /// the *normalized* mode — so the UI can dismiss it when the live key moves
    /// off it. Fails if the fields aren't populated or the mode isn't normalized
    /// (here input "Dorian" must be stored as "dorian").
    #[test]
    fn reveal_reports_generating_key() {
        let r = reveal_for(&ctx(7, "Dorian", 0.9), 0).unwrap();
        assert_eq!(r.tonic, 7);
        assert_eq!(r.mode, "dorian");
    }

    /// AC2: below the confidence threshold we stay silent rather than guess.
    /// Fails if a low-confidence reading ever produces a reveal.
    #[test]
    fn low_confidence_returns_none() {
        assert!(reveal_for(&ctx(7, "Dorian", REVEAL_MIN_CONFIDENCE - 0.01), 0).is_none());
    }

    /// AC2 boundary: exactly at the threshold is "confident enough" — pins the
    /// gate to `>=`. Fails on a `<`→`<=` mutation that would mute a valid reveal
    /// (or an `>` that would let `< threshold` through).
    #[test]
    fn confidence_at_threshold_reveals() {
        assert!(reveal_for(&ctx(7, "dorian", REVEAL_MIN_CONFIDENCE), 0).is_some());
    }

    /// A NaN confidence is treated as "not confident" (no reveal), not slipped
    /// through by `NaN < threshold == false`.
    #[test]
    fn nan_confidence_returns_none() {
        assert!(reveal_for(&ctx(7, "dorian", f32::NAN), 0).is_none());
    }

    /// #353: pins the gate's *value* to the note-gated signal's bands (the
    /// boundary tests above only pin the direction relative to the constant).
    /// Steady one-key melodic material reads ≥ ~0.67 and must reveal —
    /// raising the gate back toward 0.72 (calibrated to the old per-frame
    /// signal) turns the first assert red, which is exactly the #347 "flagship
    /// dark" regression. Semi-chromatic noodling tops out ~0.57 and must stay
    /// silent — lowering the gate below it turns the second assert red.
    #[test]
    fn gate_splits_steady_tonal_from_noodling_bands() {
        assert!(
            reveal_for(&ctx(7, "dorian", 0.67), 0).is_some(),
            "a steady-stream-class reading (0.67) must reveal"
        );
        assert!(
            reveal_for(&ctx(7, "dorian", 0.57), 0).is_none(),
            "a noodling-class reading (0.57) must stay silent"
        );
    }

    /// AC3: an unknown/unmapped mode yields nothing — never a fabricated match.
    /// Fails if the table ever falls through to a default connection.
    #[test]
    fn no_match_returns_none() {
        assert!(reveal_for(&ctx(0, "Bebop Super Locrian", 0.99), 0).is_none());
    }

    /// #364 data invariant: every curated mode key carries at least one
    /// exemplar. Downstream pickers (mirror's comparison, `reveal_for`)
    /// degrade gracefully on an empty row, but an empty row is still a
    /// content bug — it silently mutes a mode's reveals. Keep this list in
    /// sync with the `curated_for` match arms.
    #[test]
    fn every_curated_row_is_non_empty() {
        for key in [
            "major",
            "ionian",
            "minor",
            "aeolian",
            "dorian",
            "phrygian",
            "lydian",
            "mixolydian",
            "locrian",
            "harmonic minor",
            "melodic minor",
            "major pentatonic",
            "minor pentatonic",
            "blues",
        ] {
            let (display, exemplars) =
                curated_for(key).unwrap_or_else(|| panic!("{key} must stay curated"));
            assert!(
                !exemplars.is_empty(),
                "curated row {key:?} ({display}) has no exemplars"
            );
        }
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

    /// AC5 scope tripwire (not a network test): slice 1 has no LLM/network
    /// client to spy on — offline-safety holds by construction because
    /// `reveal_for` is a pure function with no I/O. This guards that property by
    /// asserting the source stays `Grounded`; if S2 ever wires an LLM path into
    /// S1 the source would flip to `LlmGrounded` and this fails, flagging the
    /// scope creep. The real "client never invoked" test arrives with S2's client.
    #[test]
    fn slice1_reveals_are_grounded_no_llm_path() {
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

    /// #253 S2 AC4: a non-empty enriched `why` replaces the line and flips the
    /// source to LlmGrounded, but never touches `concept`/`connection`; a `None`
    /// or blank rewrite keeps the curated reveal (source stays Grounded).
    #[test]
    fn apply_enriched_why_swaps_why_and_source_but_keeps_connection() {
        let base = reveal_for(&ctx(7, "dorian", 0.9), 0).unwrap();
        let enriched = apply_enriched_why(base.clone(), Some("A cooler, warmer line.".to_string()));
        assert_eq!(enriched.why, "A cooler, warmer line.");
        assert_eq!(enriched.source, RevealSource::LlmGrounded);
        assert_eq!(
            enriched.connection, base.connection,
            "connection must not change"
        );
        assert_eq!(enriched.concept, base.concept, "concept must not change");

        // None and blank both keep the curated reveal untouched.
        assert_eq!(apply_enriched_why(base.clone(), None), base);
        assert_eq!(
            apply_enriched_why(base.clone(), Some("   ".to_string())),
            base
        );
    }
}
