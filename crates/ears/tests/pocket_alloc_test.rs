//! #421 S1 AC5: the Pocket's click render — a bare [`Metronome`] as a
//! [`RenderSource`] — does not allocate. It feeds the same realtime output
//! callback as the band, so the same no-alloc law applies. Per-thread
//! counting via a const-initialised thread-local, the pattern proven on
//! the accompaniment alloc test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ears::output::{Metronome, MetronomeConfig};
use ears::output_engine::RenderSource;

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
    let mut metronome = Metronome::new(config, 48_000)
        .expect("valid config")
        .with_count_in(1);

    // Buffer allocated OUTSIDE the measured window; warm one block.
    let mut buf = vec![0.0f32; 512];
    metronome.render(&mut buf);

    COUNTING.with(|c| c.set(true));
    // Two full seconds of render — count-in bar, live bars, many clicks.
    for _ in 0..188 {
        metronome.render(&mut buf);
    }
    COUNTING.with(|c| c.set(false));

    let allocs = ALLOC_COUNT.with(|c| c.get());
    assert_eq!(allocs, 0, "the click render must never touch the heap");
}
