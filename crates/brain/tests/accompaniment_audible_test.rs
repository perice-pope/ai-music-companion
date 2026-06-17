//! Manual-verify: play the synthesized rhythm section through the real device.
//!
//! `#[ignore]`d (CI has no audio output). Run by hand to hear Slice 3a:
//!
//! ```sh
//! cargo test -p brain --test accompaniment_audible_test -- --ignored --nocapture
//! ```
//!
//! Expected: ~5 seconds of a steady ~100 BPM groove in G Mixolydian — a kick on
//! each beat, a hi-hat on the eighths, and a bass alternating G (root) and D
//! (fifth) — then silence.

use std::thread;
use std::time::Duration;

use brain::accompaniment::AccompanimentSynth;
use ears::output_engine::AudioOutput;
use groove::ClockState;
use theory::Mode;

#[test]
#[ignore = "requires a real audio output device; run with --ignored to hear it"]
fn plays_audible_groove() {
    let output = AudioOutput::start(|sample_rate| {
        let mut synth = AccompanimentSynth::new(sample_rate);
        synth.set_key(7, Mode::Mixolydian); // G Mixolydian
        synth.set_clock(ClockState {
            tempo_bpm: Some(100.0),
            swing_ratio: None,
            beat_phase: 0.0,
            confidence: 1.0,
        });
        synth
    })
    .expect("output device should be available for the manual audible test");

    println!(
        "playing G-Mixolydian groove at 100 BPM through {} Hz / {} ch — listen for ~5s",
        output.sample_rate(),
        output.channels()
    );

    thread::sleep(Duration::from_secs(5));
    output.stop();
}
