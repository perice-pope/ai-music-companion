//! #421 S1 AC5: the Pocket's click render — a bare [`Metronome`] as a
//! [`RenderSource`] — does not allocate. It feeds the same realtime output
//! callback as the band, so the same no-alloc law applies. Per-thread
//! counting via a const-initialised thread-local, the pattern proven on
//! the accompaniment alloc test.
//!
//! #445 AC5 extends the pin: the metronome now carries a click-fire
//! channel (`with_fire_channel`) whose `try_push` runs inside the render
//! path — the measured window must stay at zero allocations WITH the
//! channel attached and actually firing.
//!
//! #421 S4a AC7 extends it again: the gap-training cycle (silent bars +
//! the gap channel's drain/apply) also rides the render path, so the
//! window now covers a gapped, gap-channel-fed source too.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ears::output::{Metronome, MetronomeConfig};
use ears::output_engine::{
    click_fire_channel, pocket_gap_channel, pocket_tempo_channel, GapCycle, RenderSource,
    TempoFedMetronome,
};
use ringbuf::traits::{Consumer, Producer};

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

#[inline]
fn note_alloc() {
    let counting = COUNTING.try_with(|c| c.get()).unwrap_or(false);
    if counting {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: forwards every call verbatim to `System` (which upholds the
// `GlobalAlloc` contract); the only added work is a thread-local counter
// bump. It allocates nothing of its own and returns what `System` returns.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_alloc();
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_alloc();
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn pocket_click_render_does_not_allocate() {
    let config = MetronomeConfig {
        bpm: 96.0,
        time_signature: (4, 4),
        accent_first_beat: true,
        volume: 0.8,
    };
    // #445: the fire channel rides the render path too — attach it so
    // the measured window covers `try_push` on every click fire.
    let (fire_tx, mut fire_rx) = click_fire_channel(64);
    let metronome = Metronome::new(config, 48_000)
        .expect("valid config")
        .with_count_in(1)
        // #421 S4a: a live gap cycle — the silent-bar branch runs inside
        // the measured window (the silent live bar starts ~240 000
        // samples in, well within the 700-block window below).
        .with_gaps(1, 1)
        .with_fire_channel(fire_tx);
    // #421 S2: the CHANNEL-FED source is the real render path now —
    // the alloc law must hold through tempo pops too.
    let (mut tx, rx) = pocket_tempo_channel(16);
    let (mut gap_tx, gap_rx) = pocket_gap_channel(16);
    let mut source = TempoFedMetronome::new(metronome, rx).with_gap_channel(gap_rx);

    // Buffer allocated OUTSIDE the measured window; warm one block.
    let mut buf = vec![0.0f32; 512];
    source.render(&mut buf);

    COUNTING.with(|c| c.set(true));
    // Past the count-in→live transition (1 bar at 96 BPM = 120 000
    // samples), through steady live clicking, AND across the first silent
    // gap bar (review MF5 pattern — measure every branch the render can
    // take): 700 blocks ≈ 358 400 samples.
    for i in 0..700 {
        if i % 50 == 0 {
            let _ = tx.try_push(96.0 + i as f64 / 10.0); // live re-timing
                                                         // Re-assert the same cycle: the drain/apply path runs in the
                                                         // window without perturbing the audibility pattern.
            let _ = gap_tx.try_push(GapCycle {
                play_bars: 1,
                gap_bars: 1,
            });
        }
        source.render(&mut buf);
    }
    COUNTING.with(|c| c.set(false));

    let allocs = ALLOC_COUNT.with(|c| c.get());
    assert_eq!(allocs, 0, "the click render must never touch the heap");

    // #445: the zero-alloc window above must have been reporting fires —
    // a silently detached channel would make the assertion vacuous.
    let fires = std::iter::from_fn(|| fire_rx.try_pop()).count();
    assert!(
        fires > 0,
        "the fire channel must report clicks during the measured window"
    );
}
