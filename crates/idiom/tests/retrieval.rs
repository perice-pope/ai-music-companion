//! Integration test: embed a synthetic audio fixture, retrieve from the seed
//! corpus, and assert the confidence-gated behaviour end to end.
//!
//! These exercise the whole public surface (embed → corpus load → match) the
//! way a caller would, complementing the unit tests inside each module.

use idiom::{match_idiom, BaselineEmbedder, Corpus, IdiomEmbedder, BASELINE_DIM};

const SR: u32 = 44_100;

/// Sum-of-sines tone from MIDI `pitches`, with a soft 2nd partial — mirrors how
/// the seed corpus exemplars are synthesised, so a matching colour scores high.
fn synth(pitches: &[u8], secs: f32) -> Vec<f32> {
    let n = (secs * SR as f32) as usize;
    let mut out = vec![0.0f32; n];
    for &m in pitches {
        let f = 440.0 * 2f32.powf((m as f32 - 69.0) / 12.0);
        for (i, s) in out.iter_mut().enumerate() {
            let t = i as f32 / SR as f32;
            let tau = 2.0 * std::f32::consts::PI;
            *s += (tau * f * t).sin() + 0.3 * (tau * 2.0 * f * t).sin();
        }
    }
    out
}

#[test]
fn embeds_and_retrieves_a_plausible_match() {
    let embedder = BaselineEmbedder::new();
    let corpus = Corpus::seed().expect("seed corpus loads");
    assert_eq!(corpus.dim, BASELINE_DIM);
    assert_eq!(corpus.dim, embedder.dim());

    // A D-dorian cluster — the same colour as the "Modal / Dorian" seed.
    let query = embedder
        .embed(&synth(&[62, 65, 69, 72, 76], 1.0), SR)
        .expect("embed query");

    // Low threshold so we always get the ranked nearest neighbour.
    let matches = match_idiom(&query, &corpus, 3, 0.0).expect("retrieval");
    assert!(!matches.is_empty(), "expected at least one ranked match");

    // The nearest idiom should be the modal/dorian exemplar, and similarity
    // should be high for a colour the corpus actually contains.
    assert_eq!(matches[0].label, "Modal / Dorian colour");
    assert!(
        matches[0].similarity > 0.9,
        "self-similar colour should score high, got {}",
        matches[0].similarity
    );

    // Ranking is monotonically non-increasing.
    for w in matches.windows(2) {
        assert!(w[0].similarity >= w[1].similarity);
    }
}

#[test]
fn high_threshold_stays_silent_on_unrelated_audio() {
    let embedder = BaselineEmbedder::new();
    let corpus = Corpus::seed().unwrap();

    // White-ish noise: a flat-ish spectrum that matches no tonal idiom well.
    let mut x = 1u32;
    let noise: Vec<f32> = (0..SR as usize)
        .map(|_| {
            // Deterministic LCG so the test is reproducible.
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0
        })
        .collect();
    let query = embedder.embed(&noise, SR).unwrap();

    // With a demanding threshold the engine should decline to name anything.
    let matches = match_idiom(&query, &corpus, 3, 0.99).unwrap();
    assert!(
        matches.is_empty(),
        "engine must stay silent rather than guess, got {matches:?}"
    );
}

#[test]
fn matches_carry_full_metadata() {
    let embedder = BaselineEmbedder::new();
    let corpus = Corpus::seed().unwrap();
    let query = embedder
        .embed(&synth(&[67, 71, 74, 77, 68], 1.0), SR)
        .unwrap();

    let matches = match_idiom(&query, &corpus, 1, 0.0).unwrap();
    let top = &matches[0];
    assert!(!top.label.is_empty());
    assert!(!top.genre.is_empty());
    assert!(!top.exemplar_artist.is_empty());
    assert!(!top.era.is_empty());
    assert!(top.similarity.is_finite());
}
