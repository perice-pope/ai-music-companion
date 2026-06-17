//! Manual-verify for Slice 4a: the full follow-me chain end-to-end, audibly,
//! without the Tauri app or a microphone.
//!
//! It wires the real pieces — `AccompanimentDriver` (processing side) → lock-free
//! control channel → `AccompanimentSynthSource` (render thread) → `AudioOutput`
//! (device) — and feeds the driver synthetic onsets, exactly as the audio
//! pipeline worker will in the app. The band should lock to the implied tempo
//! and play in time.
//!
//! `#[ignore]`d (needs a real output device). Run by hand:
//!
//! ```sh
//! cargo test -p brain --test accompaniment_follow_audible_test -- --ignored --nocapture
//! ```
//!
//! Expected: a short silence, then a ~100 BPM groove (default C-major key) kicks
//! in once the clock locks onto the synthetic pulse, plays for ~4 s, then stops.

use std::thread;
use std::time::Duration;

use brain::accompaniment::{
    accompaniment_control_channel, AccompanimentDriver, AccompanimentSynth,
    AccompanimentSynthSource,
};
use ears::output_engine::AudioOutput;

#[test]
#[ignore = "requires a real audio output device; run with --ignored to hear it"]
fn band_follows_synthetic_onsets() {
    let (sender, receiver) = accompaniment_control_channel(64);
    let output = AudioOutput::start(move |sample_rate| {
        AccompanimentSynthSource::new(AccompanimentSynth::new(sample_rate), receiver)
    })
    .expect("output device should be available for the manual audible test");

    let mut driver = AccompanimentDriver::new(sender);

    println!(
        "feeding synthetic onsets at ~100 BPM through {} Hz / {} ch — band should lock and play",
        output.sample_rate(),
        output.channels()
    );

    // Feed a run of steady onsets (0.6 s apart ≈ 100 BPM). The driver advances
    // the live clock and pushes ClockState across the channel; once it locks the
    // render-thread synth starts playing. (Default key — C major — since no key
    // estimate is supplied here; key following is unit-tested separately.)
    for i in 0..10 {
        driver.observe_event(true, i as f64 * 0.6);
    }

    // Let the locked band play. It keeps the last clock and plays in real time.
    thread::sleep(Duration::from_secs(4));
    output.stop();
}
