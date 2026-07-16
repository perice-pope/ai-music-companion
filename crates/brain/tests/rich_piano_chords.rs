//! #382 — chord labels against REALISTIC piano harmonics.
//!
//! Every prior fixture used 5 partials at 1/k rolloff; a real piano carries
//! audible energy past the 10th partial, and log compression in the chroma
//! path inflates exactly that residue. With 10-partial tones this suite
//! REPRODUCED two of the VA's findings red-first (G7→"G13", cluster→an
//! invented "Dmaj7") and pins all five of her checks as regressions:
//! C major is "C", G7 is "G7", C-over-E is "C/E", one note is NO chord, a
//! cluster is NO chord. (Her real piano also decorated C→Cmaj7 and
//! E3→"E7"; this render doesn't reach those — a hotter fixture is welcome
//! if they recur.) The full seam is exercised — chroma extraction →
//! perception tracker → the label the strip would show — and every label,
//! once shown, must NEVER change for the rest of the take ("flicker and
//! lie" is half the complaint).

use brain::perception::PerceptionTracker;
use ears::chroma::ChromaExtractor;

const SR: u32 = 44_100;

fn midi_freq(m: i32) -> f32 {
    440.0 * (((m - 69) as f32) / 12.0).exp2()
}

/// Piano-ish additive tone: 10 partials, 1/k^1.1 rolloff (bright but
/// plausible for mezzo-forte in the middle register), slight per-partial
/// phase spread so peaks don't align artificially.
fn rich_render(midis: &[i32], secs: f32) -> Vec<f32> {
    let n = (secs * SR as f32) as usize;
    let mut out = vec![0.0f32; n];
    for (mi, &m) in midis.iter().enumerate() {
        let f = midi_freq(m);
        for k in 1..=10u32 {
            let amp = 1.0 / (k as f32).powf(1.1);
            let w = std::f32::consts::TAU * f * k as f32 / SR as f32;
            let phase = (mi * 7 + k as usize) as f32 * 0.61;
            for (i, o) in out.iter_mut().enumerate() {
                *o += amp * (w * i as f32 + phase).sin();
            }
        }
    }
    out
}

/// Drive the real pipeline: feed windows, take a chroma reading every
/// ~100 ms of audio, hand each to the tracker at matching timestamps, and
/// return the tracker's settled label (if any) at the end.
fn settled_label(audio: &[f32]) -> Option<String> {
    settled_label_with_bass(audio, None)
}

/// Same, feeding a sounding-bass observation each tick — the T3b path the
/// real app uses to name inversions.
fn settled_label_with_bass(audio: &[f32], bass_midi: Option<u8>) -> Option<String> {
    let mut ex = ChromaExtractor::new(SR);
    let mut tracker = PerceptionTracker::new();
    let mut now = 0.0f64;
    let mut first_label: Option<String> = None;
    for (i, w) in audio.chunks(1024).enumerate() {
        ex.feed(w);
        now += w.len() as f64 / f64::from(SR);
        // Production reads chroma at ~10 Hz, not per hop — match it so the
        // tracker's dwell counters see her cadence (review note 4).
        if i % 4 != 3 {
            continue;
        }
        if let Some(c) = ex.chroma() {
            if let Some(b) = bass_midi {
                tracker.observe_poly_bass(b, now);
            }
            tracker.observe_chroma(&c, now);
            // Stability: once a label shows, it must hold for the whole
            // take — a right answer buried in churn is still a lie.
            let label = tracker.snapshot(now).chord.map(|c| c.label);
            match (&first_label, &label) {
                (Some(seen), Some(l)) => {
                    assert_eq!(l, seen, "label churned mid-take (was {seen:?}, now {l:?})")
                }
                (None, Some(_)) => first_label = label.clone(),
                _ => {}
            }
        }
    }
    tracker.snapshot(now).chord.map(|c| c.label)
}

/// The VA's first check, two runs running: a plain C major triad must be
/// "C" — not Cmaj7 ("hearing a 7th that isn't there"), not Cmaj9. The 15th
/// partial (B) and 9th (D) are the phantoms this pins against.
#[test]
fn c_major_triad_labels_c_not_cmaj7() {
    let label = settled_label(&rich_render(&[60, 64, 67], 3.0));
    assert_eq!(
        label.as_deref(),
        Some("C"),
        "a rich C major triad must label plain C"
    );
}

/// G7 must say "G7" — not G13, G9, or F7#11/B (her actual readings). The
/// dominant is jazz-ears' bread and butter; decorating every dominant makes
/// the label useless for the one family it most needs to name.
#[test]
fn g7_labels_g7_not_g13() {
    // G3 B3 D4 F4 — a closed dominant seventh.
    let label = settled_label(&rich_render(&[55, 59, 62, 65], 3.0));
    assert_eq!(
        label.as_deref(),
        Some("G7"),
        "a rich G7 must label G7, undecorated"
    );
}

/// C major with E in the bass must read C/E (her run: one flash of
/// "G#maj9/C", then silence).
#[test]
fn c_over_e_labels_the_slash() {
    // E3 in the bass, C4 E4 G4 above; the bass path hears the E3 (T3b).
    let label = settled_label_with_bass(&rich_render(&[52, 60, 64, 67], 3.0), Some(52));
    assert_eq!(
        label.as_deref(),
        Some("C/E"),
        "first-inversion C major must read C/E"
    );
}

/// One held note must NEVER produce a chord label. E3's own partial series
/// (B = 3rd partial, G# = 5th, D = 7th) IS the E7 template — the exact
/// invention she caught twice.
#[test]
fn single_e3_never_labels_a_chord() {
    let label = settled_label(&rich_render(&[52], 3.0));
    assert_eq!(label, None, "a single rich E3 must not become a chord");
}

/// The other side of the coin (review fast-follow 2): a genuinely played
/// low-register NINTH chord must still speak. This is the discriminating
/// pin for the 7th/9th-partial HARMONIC_BLEED entries — with them
/// reverted, the low C9's own partial thicket drowns the real 9th and the
/// label dies to None. A future "cleanup" of the bleed table goes red here
/// instead of silently killing low-register extension detection.
#[test]
fn low_register_c9_still_speaks() {
    // C3 E3 G3 Bb3 D4 — a full dominant ninth, low and thick.
    let label = settled_label(&rich_render(&[48, 52, 55, 58, 62], 3.0));
    assert_eq!(
        label.as_deref(),
        Some("C9"),
        "a played low-register C9 must still be heard as C9"
    );
}

/// A chromatic mash must stay quiet (this passed both her runs — pin it so
/// the fixes above can't regress the honesty state).
#[test]
fn cluster_stays_quiet() {
    let label = settled_label(&rich_render(&[60, 61, 62, 66], 3.0));
    assert_eq!(label, None, "a cluster must stay 'hearing several notes…'");
}
