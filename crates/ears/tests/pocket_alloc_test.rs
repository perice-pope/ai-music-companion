//! #421 S1 AC5: the Pocket's click render — a bare [`Metronome`] as a
//! [`RenderSource`] — does not allocate. It feeds the same realtime output
//! callback as the band, so the same no-alloc law applies. Per-thread
//! counting via a const-initialised thread-local, the pattern proven on
//! the accompaniment alloc test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ears::output::{Metronome, MetronomeConfig};
use ears::output_engine::{pocket_tempo_channel, RenderSource, TempoFedMetronome};
use ringbuf::traits::Producer;

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
    let metronome = Metronome::new(config, 48_000)
        .expect("valid config")
        .with_count_in(1);
    // #421 S2: the CHANNEL-FED source is the real render path now —
    // the alloc law must hold through tempo pops too.
    let (mut tx, rx) = pocket_tempo_channel(16);
    let mut source = TempoFedMetronome::new(metronome, rx);

    // Buffer allocated OUTSIDE the measured window; warm one block.
    let mut buf = vec![0.0f32; 512];
    source.render(&mut buf);

    COUNTING.with(|c| c.set(true));
    // Past the count-in→live transition (1 bar at 96 BPM = 120 000
    // samples) and well into live clicking: 300 blocks ≈ 153 600 samples
    // measure the count-in accents, the transition branch, AND the
    // steady live pattern (review MF5 — the old window never left bar 1).
    for i in 0..300 {
        if i % 50 == 0 {
            let _ = tx.try_push(96.0 + i as f64 / 10.0); // live re-timing
        }
        source.render(&mut buf);
    }
    COUNTING.with(|c| c.set(false));

    let allocs = ALLOC_COUNT.with(|c| c.get());
    assert_eq!(allocs, 0, "the click render must never touch the heap");
}
