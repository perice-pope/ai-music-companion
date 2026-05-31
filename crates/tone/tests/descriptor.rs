//! Tests for the heuristic tone descriptor, room conditioning, vibrato
//! regularity, and relative-to-baseline tracking. Deterministic, no model.

use tone::{assess, features, vibrato_regularity, RoomProfile, ToneBaseline, ToneDescriptor};

const SR: u32 = 44_100;
const N: usize = 22_050;

fn sine(freq: f64) -> Vec<f32> {
    (0..N)
        .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / SR as f64).sin() as f32 * 0.7)
        .collect()
}

fn noise() -> Vec<f32> {
    let mut s: u32 = 0xC0FFEE;
    (0..N)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * 0.5
        })
        .collect()
}

#[test]
fn bright_tone_reads_bright_dark_tone_reads_warm() {
    let room = RoomProfile::neutral();
    let bright = assess(&features(&sine(3500.0), SR), None, &room);
    let dark = assess(&features(&sine(250.0), SR), None, &room);

    assert!(bright.brightness > 0.6, "bright={}", bright.brightness);
    assert!(dark.brightness < 0.4, "dark={}", dark.brightness);
    assert!(
        dark.warmth > bright.warmth,
        "dark warmth {} should exceed bright warmth {}",
        dark.warmth,
        bright.warmth
    );
}

#[test]
fn noise_reads_airy_and_uncentered_tone_reads_clear() {
    let room = RoomProfile::neutral();
    let noisy = assess(&features(&noise(), SR), None, &room);
    let tonal = assess(&features(&sine(440.0), SR), None, &room);

    assert!(
        noisy.air_noise > tonal.air_noise,
        "noisy air {} > tonal air {}",
        noisy.air_noise,
        tonal.air_noise
    );
    assert!(
        tonal.core_clarity > noisy.core_clarity,
        "tonal clarity {} > noisy clarity {}",
        tonal.core_clarity,
        noisy.core_clarity
    );
}

#[test]
fn room_conditioning_normalises_brightness() {
    // The same tone, judged in a neutral room vs a room that itself colours
    // everything bright: the bright room should pull the brightness reading down.
    let feats = features(&sine(3000.0), SR);
    let neutral = assess(&feats, None, &RoomProfile::neutral());
    let bright_room = assess(
        &feats,
        None,
        &RoomProfile {
            reference_centroid_hz: 2200.0,
            noise_floor: 0.0,
        },
    );
    assert!(
        bright_room.brightness < neutral.brightness,
        "bright-room {} should be < neutral {}",
        bright_room.brightness,
        neutral.brightness
    );
}

#[test]
fn calibrate_captures_room_reference() {
    let room = RoomProfile::calibrate(&sine(1000.0), Some(&noise()), SR);
    assert!(
        (room.reference_centroid_hz - 1000.0).abs() < 250.0,
        "reference {} near 1000",
        room.reference_centroid_hz
    );
    assert!(room.noise_floor > 0.0, "ambient noise raises the floor");
}

/// Regular sinusoidal vibrato around 440 Hz, `cycles` cycles over the contour.
fn vibrato_contour(depth_cents: f32, period_frames: f32, len: usize, jitter: f32) -> Vec<f32> {
    let mut s: u32 = 7;
    (0..len)
        .map(|i| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let j = (s >> 16) as f32 / (1u32 << 16) as f32 - 0.5;
            let phase =
                2.0 * std::f32::consts::PI * i as f32 / (period_frames * (1.0 + jitter * j));
            440.0 * 2f32.powf((depth_cents / 1200.0) * phase.sin())
        })
        .collect()
}

#[test]
fn regular_vibrato_scores_higher_than_erratic() {
    let regular = vibrato_regularity(&vibrato_contour(50.0, 12.0, 120, 0.0));
    let erratic = vibrato_regularity(&vibrato_contour(50.0, 12.0, 120, 0.9));
    assert!(regular > 0.6, "regular vibrato {} should be high", regular);
    assert!(
        regular > erratic,
        "regular {} should beat erratic {}",
        regular,
        erratic
    );
}

#[test]
fn straight_tone_and_short_contour_are_neutral() {
    let straight = vibrato_regularity(&vec![440.0; 120]); // no modulation
    assert!((straight - 0.5).abs() < f32::EPSILON, "straight = neutral");
    let short = vibrato_regularity(&[440.0, 441.0, 439.0]);
    assert!((short - 0.5).abs() < f32::EPSILON, "too short = neutral");
}

#[test]
fn baseline_tracks_and_reports_signed_deltas() {
    let warm = ToneDescriptor {
        brightness: 0.3,
        warmth: 0.8,
        air_noise: 0.1,
        core_clarity: 0.7,
        vibrato_quality: 0.5,
    };
    let mut baseline = ToneBaseline::new();
    assert!(baseline.compare(&warm).is_none(), "no baseline yet");

    baseline.update(&warm);
    // A brighter, less warm phrase than the warm baseline.
    let brighter = ToneDescriptor {
        brightness: 0.7,
        warmth: 0.4,
        ..warm
    };
    let delta = baseline.compare(&brighter).expect("baseline exists");
    assert!(delta.warmth < 0.0, "should read as less warm than baseline");
    assert!(
        delta.brightness > 0.0,
        "should read as brighter than baseline"
    );
}

#[test]
fn descriptor_serde_round_trips() {
    let d = ToneDescriptor {
        brightness: 0.6,
        warmth: 0.5,
        air_noise: 0.2,
        core_clarity: 0.8,
        vibrato_quality: 0.55,
    };
    let json = serde_json::to_string(&d).unwrap();
    let back: ToneDescriptor = serde_json::from_str(&json).unwrap();
    assert_eq!(d, back);
}
