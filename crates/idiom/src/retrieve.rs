//! Confidence-gated nearest-neighbour retrieval over the corpus.
//!
//! Cosine similarity, top-k, with a **threshold gate**: matches below the
//! configured similarity are dropped, so the engine stays *silent* rather than
//! guessing. This mirrors the project's "silence > lies" principle and the
//! design doc's grounding rule (`story-phase4-musical-relevance.md` §3B.3:
//! "below that, silence"). Retrieval is the honest "reminds me of" layer — it
//! never asserts a hard fact, only proximity.

use crate::corpus::Corpus;
use crate::error::IdiomError;
use serde::{Deserialize, Serialize};

/// A retrieved idiom match: a corpus entry the query is close to.
///
/// `similarity` is cosine similarity in `[-1, 1]` (typically `[0, 1]` for the
/// L2-normalised, non-negative-ish baseline features). It is a *proximity*
/// signal for hedged "reminds me of" phrasing, never a hard claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdiomMatch {
    /// Human-facing idiom name.
    pub label: String,
    /// Broad family / genre.
    pub genre: String,
    /// A representative artist (named, not hosted).
    pub exemplar_artist: String,
    /// Rough era.
    pub era: String,
    /// Cosine similarity to the query, in `[-1, 1]`.
    pub similarity: f32,
}

/// Cosine similarity between two equal-length vectors.
///
/// Returns `0.0` if either vector has zero magnitude. Callers must pass
/// equal-length slices; mismatched lengths are a programming error and the
/// shorter length is used (the public [`match_idiom`] guards dimension first).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Retrieve the top-`k` corpus entries above `threshold`, most similar first.
///
/// Returns an empty `Vec` when nothing clears the gate — the engine stays
/// silent rather than guessing. `threshold` is a cosine cutoff in `[-1, 1]`;
/// `k == 0` also yields an empty result. Errors only on a query/corpus
/// dimension mismatch, so the caller can detect an encoder/corpus skew.
pub fn match_idiom(
    query: &[f32],
    corpus: &Corpus,
    k: usize,
    threshold: f32,
) -> Result<Vec<IdiomMatch>, IdiomError> {
    if query.len() != corpus.dim {
        return Err(IdiomError::DimensionMismatch {
            query: query.len(),
            corpus: corpus.dim,
        });
    }
    if k == 0 {
        return Ok(Vec::new());
    }

    let mut scored: Vec<IdiomMatch> = corpus
        .entries
        .iter()
        .map(|e| IdiomMatch {
            label: e.label.clone(),
            genre: e.genre.clone(),
            exemplar_artist: e.exemplar_artist.clone(),
            era: e.era.clone(),
            similarity: cosine_similarity(query, &e.embedding),
        })
        .filter(|m| m.similarity >= threshold)
        .collect();

    // Sort by similarity descending; ties keep corpus order (stable sort).
    scored.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(k);
    Ok(scored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::CorpusEntry;

    fn entry(label: &str, embedding: Vec<f32>) -> CorpusEntry {
        CorpusEntry {
            label: label.into(),
            genre: "test".into(),
            exemplar_artist: "Test Artist".into(),
            era: "now".into(),
            embedding,
        }
    }

    fn corpus(entries: Vec<CorpusEntry>) -> Corpus {
        let dim = entries.first().map(|e| e.embedding.len()).unwrap_or(1);
        Corpus { dim, entries }
    }

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        let v = [1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_orthogonal_is_zero() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_opposite_is_minus_one() {
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_with_zero_vector_is_zero() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn returns_nearest_first() {
        let c = corpus(vec![
            entry("far", vec![0.0, 1.0]),
            entry("near", vec![1.0, 0.1]),
        ]);
        let out = match_idiom(&[1.0, 0.0], &c, 5, 0.0).unwrap();
        assert_eq!(out[0].label, "near");
        assert!(out[0].similarity >= out[1].similarity);
    }

    #[test]
    fn respects_top_k() {
        let c = corpus(vec![
            entry("a", vec![1.0, 0.0]),
            entry("b", vec![0.9, 0.1]),
            entry("c", vec![0.8, 0.2]),
        ]);
        let out = match_idiom(&[1.0, 0.0], &c, 2, 0.0).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn k_zero_returns_empty() {
        let c = corpus(vec![entry("a", vec![1.0, 0.0])]);
        assert!(match_idiom(&[1.0, 0.0], &c, 0, 0.0).unwrap().is_empty());
    }

    #[test]
    fn threshold_gate_silences_low_confidence() {
        // Query is orthogonal to the only entry → similarity 0 < threshold.
        let c = corpus(vec![entry("a", vec![0.0, 1.0])]);
        let out = match_idiom(&[1.0, 0.0], &c, 5, 0.5).unwrap();
        assert!(
            out.is_empty(),
            "below-threshold matches must be silenced, got {out:?}"
        );
    }

    #[test]
    fn threshold_admits_high_confidence() {
        let c = corpus(vec![entry("a", vec![1.0, 0.0])]);
        let out = match_idiom(&[1.0, 0.0], &c, 5, 0.5).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].similarity >= 0.5);
    }

    #[test]
    fn dimension_mismatch_errors() {
        let c = corpus(vec![entry("a", vec![1.0, 0.0, 0.0])]);
        assert!(matches!(
            match_idiom(&[1.0, 0.0], &c, 5, 0.0),
            Err(IdiomError::DimensionMismatch {
                query: 2,
                corpus: 3
            })
        ));
    }
}
