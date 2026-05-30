//! Serialise decoded note events to a standard MIDI file.
//!
//! The bytes produced here are consumed by `brain::score::midi::parse_midi_bytes`,
//! exactly like a user-supplied `.mid`, so audio import reuses the entire MIDI
//! import path downstream.

use midly::num::{u15, u24, u28, u4, u7};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};

use crate::constants::frame_to_time;
use crate::notes::NoteEvent;

/// Ticks per quarter note in the emitted file.
const TICKS_PER_BEAT: u16 = 480;
/// Tempo of the emitted file: 120 BPM (500,000 µs per quarter note).
const MICROS_PER_BEAT: u32 = 500_000;

/// Seconds-to-ticks factor at 120 BPM / 480 tpq → 960 ticks per second.
fn seconds_to_ticks(seconds: f64) -> u32 {
    let ticks_per_second = (1_000_000.0 / MICROS_PER_BEAT as f64) * TICKS_PER_BEAT as f64;
    (seconds * ticks_per_second).round().max(0.0) as u32
}

/// Build a single-track MIDI file from note events (already in model frames).
pub fn notes_to_midi(notes: &[NoteEvent]) -> Vec<u8> {
    // Flatten to absolute-tick (note-on / note-off) events.
    struct Ev {
        tick: u32,
        // off events sort before on events at the same tick
        is_on: bool,
        key: u8,
        vel: u8,
    }
    let mut events: Vec<Ev> = Vec::with_capacity(notes.len() * 2);
    for n in notes {
        let start = seconds_to_ticks(frame_to_time(n.start_frame));
        let end = seconds_to_ticks(frame_to_time(n.end_frame)).max(start + 1);
        let vel = ((n.amplitude * 127.0).round() as i32).clamp(1, 127) as u8;
        events.push(Ev {
            tick: start,
            is_on: true,
            key: n.midi,
            vel,
        });
        events.push(Ev {
            tick: end,
            is_on: false,
            key: n.midi,
            vel: 0,
        });
    }
    events.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.is_on.cmp(&b.is_on)));

    let mut track: Track = Vec::new();
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(MICROS_PER_BEAT))),
    });

    let mut prev_tick = 0u32;
    for ev in &events {
        let delta = ev.tick - prev_tick;
        prev_tick = ev.tick;
        let message = if ev.is_on {
            MidiMessage::NoteOn {
                key: u7::new(ev.key),
                vel: u7::new(ev.vel),
            }
        } else {
            MidiMessage::NoteOff {
                key: u7::new(ev.key),
                vel: u7::new(0),
            }
        };
        track.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: u4::new(0),
                message,
            },
        });
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(TICKS_PER_BEAT)),
        ),
        tracks: vec![track],
    };
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)
        .expect("writing MIDI to an in-memory Vec is infallible");
    bytes
}
