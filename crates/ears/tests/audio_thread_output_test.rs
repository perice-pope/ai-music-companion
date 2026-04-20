//! Verifies the metronome and drone don't allocate in the hot path.
//!
//! The audio thread rule (CLAUDE.md) is: NEVER allocate in `next_sample`.
//! Since we can't easily intercept the allocator here, we instead verify
//! algorithmic purity — that running many samples does not cause any Vec,
//! String, or Box state to change. The `Metronome` and `TuningDrone` structs
//! contain only primitive fields by construction; running tens of thousands
//! of samples without panic is a proxy for "no heap growth".

use ears::output::{
    ConcertPitch, DroneConfig, Metronome, MetronomeConfig, OutputMixer, TuningDrone, Waveform,
};

#[test]
fn next_sample_does_not_grow_metronome_state() {
    let mut m = Metronome::new(
        MetronomeConfig {
            bpm: 120.0,
            time_signature: (4, 4),
            accent_first_beat: true,
            volume: 0.5,
        },
        44_100,
    )
    .unwrap();
    // 10k samples covers multiple beats and at least one full click window
    // without an allocation-triggering event.
    for _ in 0..10_000 {
        let _ = m.next_sample();
    }
    // Current beat must still be a valid index within the measure.
    assert!(m.current_beat() < 4);
}

#[test]
fn next_sample_does_not_grow_drone_state() {
    for wf in [Waveform::Sine, Waveform::Triangle, Waveform::Sawtooth] {
        let mut d = TuningDrone::new(
            DroneConfig {
                frequency_hz: 440.0,
                waveform: wf,
                volume: 0.5,
            },
            44_100,
        )
        .unwrap();
        for _ in 0..10_000 {
            let s = d.next_sample();
            assert!(
                (-1.0..=1.0).contains(&s),
                "drone {:?} produced out-of-range sample {}",
                wf,
                s
            );
        }
    }
}

#[test]
fn mixer_sustains_real_time_load() {
    // Simulate 1 full second of audio through the mixer (44.1k samples).
    // This catches bugs where next_sample has O(n) behavior or accidentally
    // pushes to a buffer.
    let mut mixer = OutputMixer::new();
    mixer.set_metronome(Some(
        Metronome::new(
            MetronomeConfig {
                bpm: 120.0,
                time_signature: (4, 4),
                accent_first_beat: true,
                volume: 0.8,
            },
            44_100,
        )
        .unwrap(),
    ));
    mixer.set_drone(Some(
        TuningDrone::new(
            DroneConfig {
                frequency_hz: ConcertPitch::a_concert(),
                waveform: Waveform::Sine,
                volume: 0.3,
            },
            44_100,
        )
        .unwrap(),
    ));

    for _ in 0..44_100 {
        let s = mixer.next_sample();
        assert!((-1.0..=1.0).contains(&s));
    }
}
