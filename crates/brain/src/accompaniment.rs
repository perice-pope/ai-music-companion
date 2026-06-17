//! Local synthesized accompaniment — a backing rhythm section the player can
//! practice against, fully offline.
//!
//! [`AccompanimentSynth`] is driven by the live [`ClockState`] (tempo, beat
//! phase, lock) from [`groove::LiveClock`] and the detected key, and renders a
//! drums + bass groove into the audio output engine via [`RenderSource`]. It is
//! **synthesized** (oscillators + noise + envelopes) — no sample assets, so it
//! ships with zero licensing burden and works with no network.
//!
//! Real-time contract: [`AccompanimentSynth::render`] performs **no allocation**
//! (every voice has fixed, pre-allocated state). `set_key` / `set_clock` carry
//! control changes from the processing thread.
//!
//! This slice (3a) is the rhythm section: a kick on each beat, a hi-hat on the
//! eighths, and a bass alternating the root and the fifth of the key. The
//! harmony **pad** is a follow-up slice (3b).
//!
//! [`ClockState`]: groove::ClockState
//! [`RenderSource`]: ears::output_engine::RenderSource

use std::f64::consts::TAU;

use ears::output_engine::RenderSource;
use groove::ClockState;
use theory::Mode;

/// MIDI octave offset for the bass: pitch-class `tonic` is voiced at
/// `BASS_MIDI_BASE + tonic`, putting the root around the low-bass register
/// (e.g. tonic G → MIDI 43 = G2 ≈ 98 Hz).
const BASS_MIDI_BASE: u8 = 36; // C2
/// Master gain applied to the summed voices before clipping.
const MASTER_GAIN: f32 = 0.6;

/// Convert a MIDI note number to its equal-tempered frequency in Hz.
fn midi_to_hz(midi: u8) -> f64 {
    440.0 * 2f64.powf((midi as f64 - 69.0) / 12.0)
}

/// Per-sample exponential decay envelope, retriggerable. `level` starts at 1.0
/// on `trigger` and decays toward 0; multiplying it by a tone shapes a "hit".
#[derive(Debug, Clone, Copy)]
struct Env {
    level: f32,
    decay: f32,
}

impl Env {
    /// `decay_secs` is the time to fall ~60 dB (to 1e-3).
    fn new(decay_secs: f64, sample_rate: u32) -> Self {
        Self {
            level: 0.0,
            decay: 10f64.powf(-3.0 / (decay_secs * sample_rate as f64)) as f32,
        }
    }

    fn trigger(&mut self) {
        self.level = 1.0;
    }

    /// Return the current level and advance one sample.
    fn next(&mut self) -> f32 {
        let l = self.level;
        self.level *= self.decay;
        l
    }
}

/// A simple sine bass voice. Plays one note at a time with a decaying envelope.
#[derive(Debug, Clone, Copy)]
struct BassVoice {
    sample_rate: u32,
    phase: f64,
    phase_inc: f64,
    env: Env,
    gain: f32,
}

impl BassVoice {
    fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            phase_inc: 0.0,
            env: Env::new(0.6, sample_rate),
            gain: 0.5,
        }
    }

    /// Set the note (MIDI) the next `trigger` will sound.
    fn set_note(&mut self, midi: u8) {
        self.phase_inc = midi_to_hz(midi) / self.sample_rate as f64;
    }

    /// Restart the note's envelope (a new bass hit). Phase resets so every note
    /// attacks consistently from zero.
    fn trigger(&mut self) {
        self.phase = 0.0;
        self.env.trigger();
    }

    fn next(&mut self) -> f32 {
        let s = (TAU * self.phase).sin() as f32;
        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        s * self.env.next() * self.gain
    }
}

/// A kick drum: a sine whose pitch sweeps down from a click to a low thud,
/// shaped by a fast envelope.
#[derive(Debug, Clone, Copy)]
struct KickVoice {
    sample_rate: u32,
    phase: f64,
    /// Samples since the last trigger, for the pitch sweep.
    t: u32,
    env: Env,
    gain: f32,
}

impl KickVoice {
    const START_HZ: f64 = 120.0;
    const END_HZ: f64 = 50.0;
    /// Pitch-sweep rate (per second); higher = snappier "click → thud".
    const SWEEP: f64 = 30.0;

    fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            t: 0,
            env: Env::new(0.18, sample_rate),
            gain: 0.9,
        }
    }

    fn trigger(&mut self) {
        self.t = 0;
        self.env.trigger();
    }

    fn next(&mut self) -> f32 {
        let secs = self.t as f64 / self.sample_rate as f64;
        let hz = Self::END_HZ + (Self::START_HZ - Self::END_HZ) * (-Self::SWEEP * secs).exp();
        self.phase += hz / self.sample_rate as f64;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.t = self.t.saturating_add(1);
        (TAU * self.phase).sin() as f32 * self.env.next() * self.gain
    }
}

/// A hi-hat: filtered-ish white noise with a very fast envelope.
#[derive(Debug, Clone, Copy)]
struct HatVoice {
    rng: u32,
    env: Env,
    gain: f32,
}

impl HatVoice {
    fn new(sample_rate: u32) -> Self {
        Self {
            // Non-zero seed for the xorshift PRNG.
            rng: 0x9E37_79B9,
            env: Env::new(0.05, sample_rate),
            gain: 0.25,
        }
    }

    fn trigger(&mut self) {
        self.env.trigger();
    }

    /// xorshift32 white noise in [-1.0, 1.0]. No allocation, deterministic.
    fn noise(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn next(&mut self) -> f32 {
        self.noise() * self.env.next() * self.gain
    }
}

/// A synthesized drums + bass accompaniment driven by a live clock and key.
///
/// Feed it control updates with [`set_key`](Self::set_key) /
/// [`set_clock`](Self::set_clock) (processing thread), and pull audio with
/// [`render`](RenderSource::render). It is silent until the clock is locked.
#[derive(Debug, Clone)]
pub struct AccompanimentSynth {
    sample_rate: u32,
    tempo_bpm: Option<f32>,
    locked: bool,
    /// Beats elapsed (fractional). Advances per sample at the current tempo;
    /// integer crossings are beats, half crossings are eighths.
    beat_pos: f64,
    /// Which beat we're on, for the root/fifth bass alternation.
    beat_index: u32,
    tonic: u8,
    mode: Mode,
    bass: BassVoice,
    kick: KickVoice,
    hat: HatVoice,
}

impl AccompanimentSynth {
    /// Create a synth at the device sample rate. Defaults to C Ionian, silent
    /// (no clock yet).
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            tempo_bpm: None,
            locked: false,
            beat_pos: 0.0,
            beat_index: 0,
            tonic: 0,
            mode: Mode::Ionian,
            bass: BassVoice::new(sample_rate),
            kick: KickVoice::new(sample_rate),
            hat: HatVoice::new(sample_rate),
        }
    }

    /// Set the key the accompaniment plays in. `tonic` is a pitch class (0–11).
    pub fn set_key(&mut self, tonic: u8, mode: Mode) {
        self.tonic = tonic % 12;
        self.mode = mode;
    }

    /// Apply the latest live clock. When the clock transitions into lock, the
    /// internal beat grid aligns to the player's current beat phase so the band
    /// starts in time with them.
    pub fn set_clock(&mut self, clock: ClockState) {
        let now_locked = clock.is_locked();
        if now_locked && !self.locked {
            // Just locked — align the grid to the player and start a fresh bar.
            self.beat_pos = clock.beat_phase as f64;
            self.beat_index = 0;
        }
        self.locked = now_locked;
        self.tempo_bpm = clock.tempo_bpm;
    }

    /// MIDI note for the bass on a given beat: root on even beats, the scale's
    /// fifth degree on odd beats. Both are in-key by construction.
    fn bass_note_for_beat(&self, beat_index: u32) -> u8 {
        let degree = if beat_index.is_multiple_of(2) {
            0 // root
        } else {
            self.mode.intervals()[4] // fifth scale degree
        };
        BASS_MIDI_BASE + self.tonic + degree
    }

    /// True when the synth is producing audio (clock locked with a tempo).
    pub fn is_playing(&self) -> bool {
        self.locked && self.tempo_bpm.is_some()
    }
}

impl RenderSource for AccompanimentSynth {
    fn render(&mut self, out: &mut [f32]) {
        // Silent unless the clock is locked with a real tempo.
        let Some(bpm) = self.tempo_bpm.filter(|_| self.locked) else {
            for slot in out.iter_mut() {
                *slot = 0.0;
            }
            return;
        };

        // Beats advanced per sample.
        let inc = bpm as f64 / 60.0 / self.sample_rate as f64;
        for slot in out.iter_mut() {
            let prev = self.beat_pos;
            self.beat_pos += inc;

            // Beat boundary → kick + a bass note (root/fifth alternating).
            if self.beat_pos.floor() > prev.floor() {
                self.kick.trigger();
                let note = self.bass_note_for_beat(self.beat_index);
                self.bass.set_note(note);
                self.bass.trigger();
                self.beat_index = self.beat_index.wrapping_add(1);
            }
            // Eighth-note boundary → hi-hat.
            if (self.beat_pos * 2.0).floor() > (prev * 2.0).floor() {
                self.hat.trigger();
            }

            let mixed = self.bass.next() + self.kick.next() + self.hat.next();
            *slot = (mixed * MASTER_GAIN).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    /// A locked clock at `bpm` with `beat_phase = 0`.
    fn locked_clock(bpm: f32) -> ClockState {
        ClockState {
            tempo_bpm: Some(bpm),
            swing_ratio: None,
            beat_phase: 0.0,
            confidence: 1.0,
        }
    }

    /// Estimate a signal's fundamental (Hz) by counting zero crossings. Valid
    /// for the near-sine bass voice. Sign is independent of the (positive)
    /// envelope, so the estimate is exact regardless of decay.
    fn fundamental_hz(samples: &[f32], sample_rate: u32) -> f64 {
        let mut crossings = 0usize;
        for w in samples.windows(2) {
            if (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0) {
                crossings += 1;
            }
        }
        // Two crossings per cycle.
        crossings as f64 / 2.0 / (samples.len() as f64 / sample_rate as f64)
    }

    /// Scale pitch classes for a key.
    fn scale_pcs(tonic: u8, mode: Mode) -> Vec<u8> {
        mode.intervals().iter().map(|d| (tonic + d) % 12).collect()
    }

    /// Pitch class of a rendered fundamental in Hz.
    fn pitch_class_of(hz: f64) -> u8 {
        let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i64;
        (((midi % 12) + 12) % 12) as u8
    }

    /// Count distinct energy bursts (rising edges of 10 ms-frame RMS above
    /// `threshold`). Used to count drum hits in rendered audio.
    fn count_energy_bursts(buf: &[f32], sample_rate: u32, threshold: f32) -> usize {
        let frame = (sample_rate / 100).max(1) as usize; // 10 ms
        let mut bursts = 0;
        let mut above = false;
        for chunk in buf.chunks(frame) {
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            if rms > threshold && !above {
                bursts += 1;
                above = true;
            } else if rms <= threshold {
                above = false;
            }
        }
        bursts
    }

    /// Mute every voice except the kick (test seam — voices are private to the
    /// parent module, accessible from this child test module).
    fn solo_kick(s: &mut AccompanimentSynth) {
        s.bass.gain = 0.0;
        s.hat.gain = 0.0;
    }

    /// Mute every voice except the bass.
    fn solo_bass(s: &mut AccompanimentSynth) {
        s.kick.gain = 0.0;
        s.hat.gain = 0.0;
    }

    #[test]
    fn bass_voice_renders_at_its_note_frequency() {
        // G2 = MIDI 43 ≈ 98.0 Hz.
        let mut bass = BassVoice::new(SR);
        bass.set_note(43);
        bass.trigger();
        let mut buf = vec![0.0f32; SR as usize / 2]; // 0.5 s
        for s in buf.iter_mut() {
            *s = bass.next();
        }
        let f = fundamental_hz(&buf, SR);
        let expected = midi_to_hz(43);
        assert!(
            (f - expected).abs() < 2.0,
            "bass fundamental {f:.1} Hz should match note {expected:.1} Hz"
        );
    }

    #[test]
    fn bass_alternates_root_and_fifth_in_key() {
        // Across several keys, the bass must (a) only sound in-scale notes and
        // (b) actually voice BOTH the root and the fifth — an "always root" bug
        // would pass a weaker "in key" check.
        for (tonic, mode) in [(7u8, Mode::Mixolydian), (5u8, Mode::Dorian)] {
            let mut synth = AccompanimentSynth::new(SR);
            synth.set_key(tonic, mode);
            let allowed = scale_pcs(tonic, mode);
            let mut seen = std::collections::BTreeSet::new();
            for beat in 0..8 {
                let pc = synth.bass_note_for_beat(beat) % 12;
                assert!(
                    allowed.contains(&pc),
                    "key ({tonic},{mode:?}) beat {beat}: pitch-class {pc} not in {allowed:?}"
                );
                seen.insert(pc);
            }
            let root_pc = tonic % 12;
            let fifth_pc = (tonic + mode.intervals()[4]) % 12;
            assert!(seen.contains(&root_pc), "root {root_pc} must be voiced");
            assert!(
                seen.contains(&fifth_pc),
                "fifth {fifth_pc} must be voiced (not always the root)"
            );
        }
    }

    #[test]
    fn rendered_bass_is_in_key_through_full_render_path() {
        // End-to-end AC7: drive the real `render` path (not an isolated voice),
        // solo the bass, and analyze the RENDERED audio's fundamental during the
        // first sounded note. Catches a wrong note chosen by `bass_note_for_beat`
        // or a broken trigger/`set_note` order in `render`.
        let (tonic, mode) = (7u8, Mode::Mixolydian);
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(tonic, mode);
        synth.set_clock(locked_clock(120.0));
        solo_bass(&mut synth);

        let mut buf = vec![0.0f32; SR as usize]; // 1 s
        synth.render(&mut buf);
        // At 120 BPM the first bass note (beat 0 → root) fires at t≈0.5 s and
        // sounds until the next at t≈1.0 s. Analyze the steady middle of it.
        let a = (0.55 * SR as f32) as usize;
        let b = (0.95 * SR as f32) as usize;
        let seg = &buf[a..b];
        assert!(
            seg.iter().any(|s| s.abs() > 0.01),
            "bass should be sounding mid-beat"
        );
        let pc = pitch_class_of(fundamental_hz(seg, SR));
        assert!(
            scale_pcs(tonic, mode).contains(&pc),
            "rendered bass pitch-class {pc} not in G Mixolydian {:?}",
            scale_pcs(tonic, mode)
        );
    }

    #[test]
    fn silent_when_clock_unlocked() {
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(7, Mode::Mixolydian);
        // No clock set → unlocked.
        let mut buf = vec![9.9f32; 2048];
        synth.render(&mut buf);
        assert!(
            buf.iter().all(|s| *s == 0.0),
            "synth must be silent until the clock locks"
        );
        assert!(!synth.is_playing());
    }

    #[test]
    fn unlocking_returns_to_silence() {
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(7, Mode::Mixolydian);
        synth.set_clock(locked_clock(120.0));
        let mut buf = vec![0.0f32; SR as usize]; // 1 s of audio
        synth.render(&mut buf);
        assert!(
            buf.iter().any(|s| s.abs() > 1e-3),
            "should be audible locked"
        );

        // Clock loses lock → immediate silence.
        synth.set_clock(ClockState::SILENT);
        let mut buf2 = vec![9.9f32; 2048];
        synth.render(&mut buf2);
        assert!(
            buf2.iter().all(|s| *s == 0.0),
            "losing lock must silence the synth"
        );
    }

    #[test]
    fn silent_when_tempo_present_but_unconfident() {
        // The realistic "I hear a pulse but I'm not sure" case: a tempo is known
        // but confidence is below the lock threshold. The band must stay silent —
        // this specifically exercises the `locked` half of the render guard (a
        // tempo-only guard would wrongly play here).
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(7, Mode::Mixolydian);
        synth.set_clock(ClockState {
            tempo_bpm: Some(120.0),
            swing_ratio: None,
            beat_phase: 0.0,
            confidence: 0.3, // < LOCK_CONFIDENCE
        });
        assert!(!synth.is_playing());
        let mut buf = vec![9.9f32; SR as usize / 2];
        synth.render(&mut buf);
        assert!(
            buf.iter().all(|s| *s == 0.0),
            "tempo present but unconfident must be silent (not locked)"
        );
    }

    #[test]
    fn audible_and_in_range_when_locked() {
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(7, Mode::Mixolydian);
        synth.set_clock(locked_clock(120.0));
        let mut buf = vec![0.0f32; SR as usize]; // 1 s
        synth.render(&mut buf);
        assert!(
            buf.iter().any(|s| s.abs() > 0.05),
            "locked synth should produce audible output"
        );
        assert!(
            buf.iter().all(|s| (-1.0..=1.0).contains(s)),
            "output must stay within [-1, 1]"
        );
    }

    #[test]
    fn kick_sounds_on_each_beat() {
        // Audio-level: solo the kick and count actual energy bursts in the
        // rendered output. At 120 BPM beats fall at t≈0.5 s and t≈1.0 s, so a
        // 1.1 s render must contain exactly 2 kick bursts. A silenced kick (or a
        // no-op trigger) yields 0 bursts and fails — unlike a beat-counter check.
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(0, Mode::Ionian);
        synth.set_clock(locked_clock(120.0));
        solo_kick(&mut synth);
        let mut buf = vec![0.0f32; (SR as f32 * 1.1) as usize];
        synth.render(&mut buf);
        let bursts = count_energy_bursts(&buf, SR, 0.02);
        assert_eq!(
            bursts, 2,
            "kick should sound twice in 1.1 s at 120 BPM, got {bursts} bursts"
        );
    }

    #[test]
    fn faster_tempo_produces_more_kicks() {
        // Audio-level rate check: 180 BPM yields more kick bursts than 60 BPM in
        // the same window. Guards against `render` ignoring the tempo.
        let render_kicks = |bpm: f32| {
            let mut s = AccompanimentSynth::new(SR);
            s.set_key(0, Mode::Ionian);
            s.set_clock(locked_clock(bpm));
            solo_kick(&mut s);
            let mut buf = vec![0.0f32; (SR as f32 * 2.0) as usize]; // 2 s
            s.render(&mut buf);
            count_energy_bursts(&buf, SR, 0.02)
        };
        let slow = render_kicks(60.0); // ~2 kicks in 2 s
        let fast = render_kicks(180.0); // ~6 kicks in 2 s
        assert!(
            fast > slow,
            "180 BPM ({fast} kicks) should out-pace 60 BPM ({slow} kicks)"
        );
    }

    #[test]
    fn tempo_change_while_locked_is_continuous() {
        // Changing tempo while staying locked must NOT reset the beat grid (no
        // glitch), and the new rate must take effect. §6 "tempo changes mid-play".
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(0, Mode::Ionian);
        synth.set_clock(locked_clock(120.0));
        let mut buf = vec![0.0f32; SR as usize / 4]; // 0.25 s → 0.5 beat @120
        synth.render(&mut buf);
        let pos_before = synth.beat_pos;
        assert!(pos_before > 0.0);

        synth.set_clock(locked_clock(180.0)); // still locked, faster
        assert!(
            (synth.beat_pos - pos_before).abs() < 1e-9,
            "tempo change while locked must not reset the grid (was {pos_before}, now {})",
            synth.beat_pos
        );

        let mut buf2 = vec![0.0f32; SR as usize / 4]; // another 0.25 s @180 → 0.75 beat
        synth.render(&mut buf2);
        let advanced = synth.beat_pos - pos_before;
        assert!(
            (advanced - 0.75).abs() < 0.05,
            "0.25 s at 180 BPM should advance 0.75 beats, got {advanced:.3}"
        );
    }

    #[test]
    fn synth_is_sample_rate_independent() {
        // The bass fundamental and the beat rate must be the same regardless of
        // the device sample rate (§6) — proving phase_inc / beat inc divide by sr.
        for sr in [44_100u32, 48_000] {
            let mut bass = BassVoice::new(sr);
            bass.set_note(43); // G2
            bass.trigger();
            let mut buf = vec![0.0f32; sr as usize / 2];
            for s in buf.iter_mut() {
                *s = bass.next();
            }
            let f = fundamental_hz(&buf, sr);
            assert!(
                (f - midi_to_hz(43)).abs() < 2.0,
                "bass fundamental {f:.1} Hz wrong at {sr} Hz"
            );

            let mut synth = AccompanimentSynth::new(sr);
            synth.set_key(0, Mode::Ionian);
            synth.set_clock(locked_clock(120.0));
            let mut sbuf = vec![0.0f32; sr as usize]; // 1 s
            synth.render(&mut sbuf);
            assert!(
                (synth.beat_pos - 2.0).abs() < 0.05,
                "120 BPM should advance 2 beats/s at {sr} Hz, got {}",
                synth.beat_pos
            );
        }
    }

    #[test]
    fn aligns_to_player_phase_on_lock() {
        // When the clock locks mid-beat, the grid should adopt the player's beat
        // phase so the band starts in time, not from zero.
        let mut synth = AccompanimentSynth::new(SR);
        synth.set_key(0, Mode::Ionian);
        let clock = ClockState {
            tempo_bpm: Some(120.0),
            swing_ratio: None,
            beat_phase: 0.5,
            confidence: 1.0,
        };
        synth.set_clock(clock);
        assert!(
            (synth.beat_pos - 0.5).abs() < 1e-6,
            "grid should align to the player's beat phase (0.5) on lock, got {}",
            synth.beat_pos
        );
    }
}
