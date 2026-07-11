//! Verifies the chroma path (`feed` → `chroma`) does not allocate after
//! construction — the same contract (and the same counting-allocator
//! technique) as `pitch_alloc_test.rs` (#245): the extractor runs in the
//! processing-thread analysis loop, and a per-frame allocation there is a
//! latency hiccup waiting to happen.
//!
//! Counting is per-thread (thread-local flag) so libtest's own harness
//! allocations can't make this flaky.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ears::chroma::ChromaExtractor;

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

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_alloc();
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_alloc();
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_alloc();
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// #349 T1b AC (zero-alloc gate): after construction, an extended
/// feed → chroma loop — warm-up, voiced audio, silence, and a reset —
/// performs zero allocator traffic. Fails if the ring buffer, the Goertzel
/// bank, or the fold ever grows or boxes per call.
#[test]
fn chroma_loop_never_allocates_after_construction() {
    const SR: u32 = 44_100;
    let mut extractor = ChromaExtractor::new(SR);

    // A crude sawtooth-ish chord window (values only need to be non-trivial;
    // built OUTSIDE the measured region).
    let window: Vec<f32> = (0..1024)
        .map(|i| {
            let t = i as f32 / SR as f32;
            (t * 220.0 * std::f32::consts::TAU).sin() * 0.5
                + (t * 277.18 * std::f32::consts::TAU).sin() * 0.4
                + (t * 329.63 * std::f32::consts::TAU).sin() * 0.4
        })
        .collect();
    let silence = vec![0.0f32; 1024];

    COUNTING.with(|c| c.set(true));
    // Warm-up (first call included — a lazy init would be caught here).
    for i in 0..64 {
        extractor.feed(&window);
        if i % 4 == 0 {
            let _ = extractor.chroma();
        }
    }
    // Silence and transition back to sound.
    for _ in 0..16 {
        extractor.feed(&silence);
        let _ = extractor.chroma();
    }
    extractor.reset();
    for _ in 0..16 {
        extractor.feed(&window);
        let _ = extractor.chroma();
    }
    COUNTING.with(|c| c.set(false));

    let count = ALLOC_COUNT.with(|c| c.get());
    assert_eq!(
        count, 0,
        "chroma path performed {count} allocator operations; the analysis \
         loop must be allocation-free after construction"
    );
}
