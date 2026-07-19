//! #382 diagnosis harness: trace what the tracker sees on the
//! bass-voicing fixture, reading by reading.
//! `cargo run -p brain --example trace_382`
use brain::perception::PerceptionTracker;
use ears::chroma::ChromaExtractor;

fn main() {
    let path = format!(
        "{}/tests/fixtures/real_piano/c-major-over-c3.wav",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap();
    let data_pos = bytes.windows(4).position(|w| w == b"data").unwrap();
    let audio: Vec<f32> = bytes[data_pos + 8..]
        .chunks_exact(2)
        .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
        .collect();

    let mut ex = ChromaExtractor::new(44_100);
    let mut tracker = PerceptionTracker::new();
    let mut now = 0.0f64;
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    for (i, w) in audio.chunks(1024).enumerate() {
        ex.feed(w);
        now += w.len() as f64 / 44_100.0;
        if i % 4 != 3 {
            continue;
        }
        if let Some(c) = ex.chroma() {
            tracker.observe_poly_bass(48, now);
            tracker.observe_chroma(&c, now);
            let label = tracker
                .snapshot(now)
                .chord
                .map(|ch| ch.label)
                .unwrap_or_else(|| "-".into());
            let mut pcs: Vec<(usize, f32)> = c.iter().copied().enumerate().collect();
            pcs.sort_by(|a, b| b.1.total_cmp(&a.1));
            let top: Vec<String> = pcs
                .iter()
                .take(5)
                .map(|(pc, v)| format!("{}={:.2}", NAMES[*pc], v))
                .collect();
            println!("{now:5.2}s  label={label:<6} {}", top.join(" "));
        }
    }
}
