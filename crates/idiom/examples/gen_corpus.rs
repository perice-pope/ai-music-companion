//! Regenerate the seed `data/corpus.json` from synthetic exemplar audio.
//!
//! Run with `cargo run -p idiom --example gen_corpus`. This is a developer
//! tool, not part of the library: it synthesises a short, characteristic tone
//! cluster for each seed idiom (a chord / scale colour that stands in for the
//! style), embeds it with [`BaselineEmbedder`], and writes the corpus JSON.
//!
//! When a MERT-class encoder replaces the baseline, regenerate the corpus from
//! *real* reference recordings with the new encoder (see crate docs); this
//! synthetic generator is only how the *seed* embeddings are produced so they
//! are real-shaped and consistent with query embeddings.

use idiom::{BaselineEmbedder, IdiomEmbedder};
use serde_json::json;

const SAMPLE_RATE: u32 = 44_100;
const SECS: f32 = 1.0;

/// One seed idiom: metadata plus the MIDI pitches whose blend characterises it.
struct Seed {
    label: &'static str,
    genre: &'static str,
    exemplar_artist: &'static str,
    era: &'static str,
    /// MIDI note numbers sounded together as the exemplar colour.
    pitches: &'static [u8],
}

/// Synthesise a blended tone from `pitches` (sum of sines + a soft 2nd partial).
fn synth(pitches: &[u8]) -> Vec<f32> {
    let n = (SECS * SAMPLE_RATE as f32) as usize;
    let mut out = vec![0.0f32; n];
    for &m in pitches {
        let f = 440.0 * 2f32.powf((m as f32 - 69.0) / 12.0);
        for (i, s) in out.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE as f32;
            let tau = 2.0 * std::f32::consts::PI;
            *s += (tau * f * t).sin() + 0.3 * (tau * 2.0 * f * t).sin();
        }
    }
    out
}

fn main() {
    // Seed idioms across several genres (jazz-deep, per the design doc), each
    // characterised by a representative chord / scale colour.
    let seeds = [
        Seed {
            label: "Bebop dominant line",
            genre: "jazz",
            exemplar_artist: "Charlie Parker",
            era: "1940s-50s",
            // G7 with bebop chromaticism: G B D F + Ab passing colour.
            pitches: &[67, 71, 74, 77, 68],
        },
        Seed {
            label: "Modal / Dorian colour",
            genre: "jazz",
            exemplar_artist: "Miles Davis",
            era: "1950s-60s",
            // D dorian cluster: D F A C E.
            pitches: &[62, 65, 69, 72, 76],
        },
        Seed {
            label: "Blues / mixolydian",
            genre: "blues",
            exemplar_artist: "B.B. King",
            era: "1950s-70s",
            // E7 with blue third/seventh: E G# D + b3 G + b5 Bb.
            pitches: &[64, 68, 74, 67, 70],
        },
        Seed {
            label: "Bossa nova major-7th",
            genre: "bossa nova",
            exemplar_artist: "Antonio Carlos Jobim",
            era: "1960s",
            // Cmaj7 with 9th: C E G B D.
            pitches: &[60, 64, 67, 71, 74],
        },
        Seed {
            label: "Romantic classical tonality",
            genre: "classical",
            exemplar_artist: "Frederic Chopin",
            era: "1800s",
            // C major triad, wide voicing C E G C.
            pitches: &[48, 64, 67, 72],
        },
        Seed {
            label: "Whole-tone / impressionist",
            genre: "classical",
            exemplar_artist: "Claude Debussy",
            era: "1900s",
            // Whole-tone fragment: C D E F# G# A#.
            pitches: &[60, 62, 64, 66, 68, 70],
        },
    ];

    let embedder = BaselineEmbedder::new();
    let entries: Vec<_> = seeds
        .iter()
        .map(|s| {
            let audio = synth(s.pitches);
            let embedding = embedder.embed(&audio, SAMPLE_RATE).expect("embed seed");
            json!({
                "label": s.label,
                "genre": s.genre,
                "exemplar_artist": s.exemplar_artist,
                "era": s.era,
                "embedding": embedding,
            })
        })
        .collect();

    let corpus = json!({ "dim": embedder.dim(), "entries": entries });
    let pretty = serde_json::to_string_pretty(&corpus).expect("serialize corpus");
    println!("{pretty}");
}
