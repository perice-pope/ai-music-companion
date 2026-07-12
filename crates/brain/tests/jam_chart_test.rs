//! #349 T4a AC1/AC2 — the jam chart end to end: rendered ROOM audio
//! (chords plus a percussion noise bed) through the real chroma front end,
//! the real perception tracker, and the real chart recorder. No audio
//! files, no network — everything synthesized in-test, deterministic.

use brain::chord_chart::ChartRecorder;
use brain::perception::PerceptionTracker;
use ears::chroma::ChromaExtractor;

const SR: u32 = 44_100;

fn midi_freq(m: i32) -> f32 {
    440.0 * (((m - 69) as f32) / 12.0).exp2()
}

/// Deterministic pseudo-noise (xorshift) — the percussion bed.
struct Noise(u64);
impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        ((x as f64 / u64::MAX as f64) as f32 - 0.5) * 2.0
    }
}

/// Render `secs` of a chord (5 partials per tone, 1/k rolloff) over a
/// percussion-ish bed: broadband noise bursts on every half-second.
fn render_mixed(midis: &[i32], secs: f32, noise: &mut Noise) -> Vec<f32> {
    let n = (secs * SR as f32) as usize;
    let mut out = vec![0.0f32; n];
    for &m in midis {
        let f = midi_freq(m);
        for k in 1..=5u32 {
            let amp = 1.0 / k as f32;
            let w = std::f32::consts::TAU * f * k as f32 / SR as f32;
            for (i, o) in out.iter_mut().enumerate() {
                *o += amp * (w * i as f32).sin();
            }
        }
    }
    // Percussion bed: a 60 ms noise burst at each half-second boundary,
    // loud enough to be real (≈20% of a partial), broadband so it lands in
    // every chroma bin at once.
    let burst = (0.06 * SR as f32) as usize;
    let half = (0.5 * SR as f32) as usize;
    let mut at = 0;
    while at < n {
        for sample in out.iter_mut().take((at + burst).min(n)).skip(at) {
            *sample += 0.6 * noise.next();
        }
        at += half;
    }
    out
}

/// Drive the full stack the way the session worker does: feed windows,
/// compute chroma every ~93 ms, snapshot + chart every other reading.
fn play(
    audio: &[f32],
    t0: f64,
    extractor: &mut ChromaExtractor,
    perception: &mut PerceptionTracker,
    chart: &mut ChartRecorder,
) -> f64 {
    let mut t = t0;
    let mut since = 0usize;
    let mut tick = 0usize;
    for w in audio.chunks(1024) {
        extractor.feed(w);
        since += w.len();
        t += w.len() as f64 / SR as f64;
        if since >= 4096 {
            since = 0;
            if let Some(c) = extractor.chroma() {
                perception.observe_chroma(&c, t);
            }
            tick += 1;
            if tick.is_multiple_of(2) {
                let snap = perception.snapshot(t);
                chart.observe(&snap, t);
            }
        }
    }
    t
}

/// #349 T4a AC1: a synthetic 4-chord "record" (percussion bed and all)
/// yields exactly those 4 labels, in order, timestamped, with real
/// confidences — and each entry carries the quality key the tap-to-row
/// bridge needs. Fails if the front end, tracker, or recorder mangles the
/// sequence or the bed defeats the honesty gates.
#[test]
fn a_four_chord_record_charts_in_order() {
    let mut noise = Noise::new_seeded();
    let mut extractor = ChromaExtractor::new(SR);
    let mut perception = PerceptionTracker::new();
    let mut chart = ChartRecorder::new();

    // C — Am — F — G, close voicings around C4, 2 s each.
    let progression: [(&[i32], &str); 4] = [
        (&[48, 52, 55], "C"),
        (&[45, 48, 52], "Am"),
        (&[41, 45, 48], "F"),
        (&[43, 47, 50], "G"),
    ];
    let mut t = 0.0;
    for (midis, _) in progression {
        let audio = render_mixed(midis, 2.0, &mut noise);
        t = play(&audio, t, &mut extractor, &mut perception, &mut chart);
    }

    let labels: Vec<&str> = chart
        .entries()
        .iter()
        .filter(|e| !e.unresolved)
        .map(|e| e.label.as_str())
        .collect();
    assert_eq!(
        labels,
        ["C", "Am", "F", "G"],
        "full chart: {:?}",
        chart.entries()
    );
    for e in chart.entries().iter().filter(|e| !e.unresolved) {
        assert!(
            e.confidence > 0.0,
            "confidence dots need real values: {e:?}"
        );
        assert!(e.quality.is_some(), "tap-to-row needs the quality: {e:?}");
    }
    let times: Vec<f64> = chart.entries().iter().map(|e| e.at_secs).collect();
    assert!(times.windows(2).all(|w| w[0] < w[1]), "timestamps ordered");
}

/// #349 T4a AC2: a dense atonal mix charts HONEST unresolved entries —
/// never a fabricated label. Fails if the vocabulary gate or the recorder
/// upgrades mush to a name.
#[test]
fn an_atonal_mix_charts_honestly_unresolved() {
    let mut noise = Noise::new_seeded();
    let mut extractor = ChromaExtractor::new(SR);
    let mut perception = PerceptionTracker::new();
    let mut chart = ChartRecorder::new();

    // A chromatic cluster no template names: C, C#, D, Eb, E together.
    let audio = render_mixed(&[48, 49, 50, 51, 52], 3.0, &mut noise);
    play(&audio, 0.0, &mut extractor, &mut perception, &mut chart);

    assert!(
        !chart.entries().is_empty(),
        "several notes clearly sounding must be acknowledged"
    );
    assert!(
        chart.entries().iter().all(|e| e.unresolved),
        "no fabricated labels for an atonal mix: {:?}",
        chart.entries()
    );
}

impl Noise {
    fn new_seeded() -> Self {
        Noise(0x9E37_79B9_7F4A_7C15)
    }
}
