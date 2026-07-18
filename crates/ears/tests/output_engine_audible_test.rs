//! Manual-verify: play an audible metronome through the real output device.
//!
//! This test opens the default output device, so it is `#[ignore]`d by default
//! (CI runners have no audio output, exactly like the input pipeline). Run it by
//! hand to confirm Slice 1 produces real sound:
//!
//! ```sh
//! cargo test -p ears --test output_engine_audible_test -- --ignored --nocapture
//! ```
//!
//! Expected: ~3 seconds of a 120 BPM metronome click (accented downbeat), then
//! silence when the engine is dropped.

use std::thread;
use std::time::Duration;

use ears::output::{Metronome, MetronomeConfig, OutputMixer};
use ears::output_engine::AudioOutput;

#[test]
#[ignore = "requires a real audio output device; run with --ignored to hear it"]
fn plays_audible_metronome() {
    let output = AudioOutput::start(|sample_rate| {
        let metronome = Metronome::new(
            MetronomeConfig {
                bpm: 120.0,
                time_signature: (4, 4),
                accent_first_beat: true,
                volume: 0.8,
            },
            sample_rate,
        )
        .expect("valid metronome config");
        let mut mixer = OutputMixer::new();
        mixer.set_metronome(Some(metronome));
        mixer
    })
    .expect("output device should be available for the manual audible test");

    println!(
        "playing metronome: {} Hz, {} channels — listen for ~3s of clicks",
        output.sample_rate(),
        output.channels()
    );

    thread::sleep(Duration::from_secs(3));
    output.stop(); // joins both threads and releases the device -> silence
}

/// #421 S1: The Pocket's ACTUAL source — a bare [`Metronome`] as a
/// [`RenderSource`] with a count-in — audibly verified. Run with
/// `cargo test -p ears --test output_engine_audible_test -- --ignored --nocapture`
/// and listen: one bar of high call-off clicks, then accent-on-1.
#[test]
#[ignore = "plays audio on the default output device — manual verify"]
fn plays_the_pocket_click_with_count_in() {
    use ears::output::{Metronome, MetronomeConfig};
    let output = ears::output_engine::AudioOutput::start(|sample_rate| {
        Metronome::new(
            MetronomeConfig {
                bpm: 96.0,
                time_signature: (4, 4),
                accent_first_beat: true,
                volume: 0.8,
            },
            sample_rate,
        )
        .expect("valid config")
        .with_count_in(1)
    })
    .expect("output device");
    std::thread::sleep(std::time::Duration::from_secs(5));
    output.stop();
}
