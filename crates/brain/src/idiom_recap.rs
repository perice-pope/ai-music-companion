//! Offline idiom analysis for the session recap.
//!
//! This module wires the on-device idiom engine (`crates/idiom`) into the
//! practice → recap flow. At **session scope** (never per audio frame) it
//! embeds the session's captured audio with [`BaselineEmbedder`] and retrieves
//! the nearest known idioms from the bundled [`Corpus`], producing a small,
//! confidence-gated list of [`IdiomMatch`] the recap can surface as grounded
//! "that phrase has a bebop flavour" notes.
//!
//! ## Fully offline — no network, ever
//!
//! Everything here runs entirely on-device. The embedder is classic DSP, the
//! corpus is bundled into the `idiom` crate at build time (`include_str!` of
//! `crates/idiom/data/corpus.json`), and retrieval is local cosine search.
//! There is **no HTTP client, no API key, and no internet dependency** on this
//! path — it is a key offline differentiator and the basis for the recap's
//! offline fallback. The accompanying tests assert this by running the whole
//! path with no [`crate::coaching::HttpClient`] in sight.
//!
//! ## Off the real-time audio thread
//!
//! Idiom embedding allocates and runs an FFT, so it must **never** touch the
//! <25 ms cpal hot path described in `CLAUDE.md`. It is called once, at the
//! session boundary, from the recap-building path on the processing/worker
//! side — the same place the LLM recap is assembled. Allocation here is fine.
//!
//! ## Grounded + confidence-gated ("silence > lies")
//!
//! Matches below the engine's similarity threshold are dropped: if the session
//! audio doesn't actually sit near a known idiom, we surface nothing rather
//! than guess. A returned [`IdiomMatch::similarity`] is a *real* proximity in
//! embedding space — a hedged "reminds me of", never an asserted fact. The
//! recap (and the coaching prompt) treat these as grounded input to hedge
//! around, never as ground truth.

use idiom::{match_idiom, BaselineEmbedder, Corpus, IdiomEmbedder};

// Re-export the engine's match shape so the rest of `brain` (and the Tauri
// shell / frontend, which mirror brain's serde shapes) depend on one type.
pub use idiom::IdiomMatch;

/// Cosine-similarity gate below which a match is silenced. Mirrors the
/// example threshold in the `idiom` crate docs: only surface a flavour when
/// the audio genuinely sits near a known idiom; otherwise stay quiet.
pub const IDIOM_SIMILARITY_THRESHOLD: f32 = 0.6;

/// How many idiom flavours to surface at most. The recap is a quiet, single
/// "reminds me of" line — a short list keeps it honest and uncluttered.
pub const IDIOM_TOP_K: usize = 2;

/// Analyse `samples` (mono PCM at `sample_rate` Hz) for idiom flavour, fully
/// offline, returning a confidence-gated list of nearest known idioms.
///
/// Returns an empty `Vec` (the engine stays *silent*) when:
/// - the input is empty or otherwise unembeddable,
/// - the bundled corpus fails to load (should never happen — it's validated at
///   build time), or
/// - nothing clears [`IDIOM_SIMILARITY_THRESHOLD`].
///
/// Errors are deliberately swallowed into "silence": a recap must never fail
/// because the optional idiom layer hit a snag. This is the offline, off-hot-
/// path entry point the recap builder calls at session scope.
pub fn analyze_idioms(samples: &[f32], sample_rate: u32) -> Vec<IdiomMatch> {
    if samples.is_empty() {
        return Vec::new();
    }

    // Bundled, build-time-validated corpus — no file IO, no network.
    let corpus = match Corpus::seed() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let embedder = BaselineEmbedder::new();
    let query = match embedder.embed(samples, sample_rate) {
        Ok(q) => q,
        Err(_) => return Vec::new(),
    };

    // Confidence-gated top-k. `match_idiom` only errors on a dimension skew
    // between query and corpus (encoder/corpus mismatch); treat that as
    // "stay silent" too rather than break the recap.
    match_idiom(&query, &corpus, IDIOM_TOP_K, IDIOM_SIMILARITY_THRESHOLD).unwrap_or_default()
}

/// One quiet, grounded, hedged line per idiom match, for the offline fallback
/// recap (which has no LLM to phrase it). Names the genre and an exemplar
/// artist so the note is concrete, and hedges ("has a … flavour") because the
/// match is proximity, not a claim.
///
/// Returns `None` for an empty list so callers can skip the line entirely.
pub fn describe_idioms(matches: &[IdiomMatch]) -> Option<String> {
    let top = matches.first()?;
    Some(format!(
        "What you played has a {} flavour — it shares a neighbourhood with {} ({}).",
        top.genre, top.label, top.exemplar_artist
    ))
}

/// A compact, label-led block of the gated idiom matches for the LLM recap
/// prompt. The model is told these are *grounded* proximity signals it may
/// hedge around, never invented and never to be stated as hard fact. Returns
/// an empty string when there's nothing to say (the prompt stays silent).
pub fn idiom_prompt_block(matches: &[IdiomMatch]) -> String {
    if matches.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = matches
        .iter()
        .map(|m| {
            format!(
                "  - {} ({}, exemplar: {}, era: {}) — similarity {:.2}",
                m.label, m.genre, m.exemplar_artist, m.era, m.similarity
            )
        })
        .collect();
    format!(
        "\n\nIdiom proximity (GROUNDED INPUT — these are real audio-similarity \
         signals computed on-device, not assertions; you MAY gently hedge around \
         them as \"reminds me of\" / \"has a … flavour\", but NEVER state them as \
         hard fact and NEVER invent others):\n{}",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure sine tone, used as deterministic, embeddable audio.
    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f32) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn empty_audio_yields_no_matches() {
        assert!(analyze_idioms(&[], 44_100).is_empty());
    }

    #[test]
    fn invalid_sample_rate_yields_no_matches() {
        // The embedder rejects sr == 0; we swallow that into silence.
        assert!(analyze_idioms(&sine(440.0, 0.5, 44_100), 0).is_empty());
    }

    #[test]
    fn runs_fully_offline_against_bundled_corpus() {
        // No HttpClient, no network, no file IO beyond the build-time corpus.
        // A real tone always embeds; whether it clears the gate depends on the
        // corpus, but the path itself must run and return a well-formed list.
        let matches = analyze_idioms(&sine(261.63, 1.0, 44_100), 44_100);
        for m in &matches {
            assert!(
                m.similarity >= IDIOM_SIMILARITY_THRESHOLD,
                "every surfaced match must clear the confidence gate, got {m:?}"
            );
            assert!(!m.label.is_empty());
            assert!(!m.genre.is_empty());
        }
        assert!(
            matches.len() <= IDIOM_TOP_K,
            "must not exceed top-k, got {}",
            matches.len()
        );
    }

    #[test]
    fn idiom_matches_are_corpus_aligned() {
        // A tone whose embedding sits near a corpus entry should surface that
        // entry. We assert the *gating contract* holds across the bundled
        // corpus: pick the tone that most resembles a corpus exemplar by
        // scanning a few pitches, and require at least one to produce a match.
        let pitches = [196.0, 261.63, 329.63, 392.0, 440.0, 523.25];
        let any = pitches
            .iter()
            .any(|&f| !analyze_idioms(&sine(f, 1.0, 44_100), 44_100).is_empty());
        assert!(
            any,
            "at least one exemplar-like tone must clear the gate against the \
             bundled corpus (the corpus is built from baseline-embedded tones)"
        );
    }

    #[test]
    fn describe_idioms_is_none_when_empty() {
        assert!(describe_idioms(&[]).is_none());
    }

    #[test]
    fn describe_idioms_hedges_and_names_exemplar() {
        let m = IdiomMatch {
            label: "Bebop line".into(),
            genre: "jazz".into(),
            exemplar_artist: "Charlie Parker".into(),
            era: "1940s-50s".into(),
            similarity: 0.82,
        };
        let line = describe_idioms(std::slice::from_ref(&m)).unwrap();
        assert!(line.contains("flavour"), "must hedge: {line}");
        assert!(line.contains("jazz"));
        assert!(line.contains("Charlie Parker"));
    }

    #[test]
    fn prompt_block_is_empty_for_no_matches() {
        assert!(idiom_prompt_block(&[]).is_empty());
    }

    #[test]
    fn prompt_block_marks_grounded_and_forbids_invention() {
        let m = IdiomMatch {
            label: "Modal / Dorian colour".into(),
            genre: "jazz".into(),
            exemplar_artist: "Miles Davis".into(),
            era: "1960s".into(),
            similarity: 0.7,
        };
        let block = idiom_prompt_block(std::slice::from_ref(&m));
        assert!(block.contains("GROUNDED INPUT"));
        assert!(block.contains("NEVER invent"));
        assert!(block.contains("Modal / Dorian colour"));
    }
}
