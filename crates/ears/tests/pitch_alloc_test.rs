//! Verifies the analysis path (samples → `AudioEvent`) does not allocate.
//!
//! The audio-thread rule (CLAUDE.md) is: NEVER allocate in the real-time path.
//! `PitchDetector::detect()` documents "zero heap allocations" — but hotspot
//! #245 found it allocating a fresh `String` for every detected frame's
//! `note_name`. This test installs a global allocator that counts allocations
//! and asserts the steady-state detect loop — voiced frames (which build
//! `NoteInfo`), silence frames, and the silence→sound transition — performs
//! **zero** of them, so the regression can't come back.
//!
//! Counting is **per-thread**: the `GlobalAlloc` is process-global and runs on
//! every thread (including libtest's own harness thread, which allocates
//! concurrently), so a global flag would count unrelated allocations and the
//! test would be flaky across platforms. A thread-local flag scopes the count to
//! exactly the thread under measurement. The flag/counter use `const`-initialised
//! `Cell`s, so reading them inside the allocator never itself allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    /// When true on the current thread, allocations on this thread are counted.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    /// Allocations observed on the current thread while `COUNTING` was true.
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// A pass-through allocator that counts allocations on the measuring thread.
struct CountingAlloc;

#[inline]
fn note_alloc() {
    // `try_with` so an allocation during TLS teardown can't panic.
    let counting = COUNTING.try_with(|c| c.get()).unwrap_or(false);
    if counting {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: `CountingAlloc` forwards every call verbatim to the `System`
// allocator (which upholds the `GlobalAlloc` contract) and adds only a
// thread-local counter bump. It allocates no memory of its own and returns
// exactly what `System` returns, so all pointer/layout invariants are preserved.
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

use ears::pitch::{generate_sine, PitchConfig, PitchDetector};

#[test]
fn detect_does_not_allocate_in_steady_state() {
    let mut detector = PitchDetector::new(PitchConfig::default()).unwrap();

    // Pre-generate every input OUTSIDE the measured window; detect() must not
    // allocate even on the voiced path that builds `NoteInfo`.
    let tone = generate_sine(440.0, 44100, detector.window_size());
    let silence = vec![0.0f32; detector.window_size()];

    // Warm up: exercise the voiced, silence, and re-entry paths once each.
    let warm = detector.detect(&tone);
    assert_eq!(
        warm.note_info
            .as_ref()
            .expect("440 Hz sine must yield note info")
            .note_name,
        "A",
        "guard must measure the real voiced path, not a silent no-op"
    );
    detector.detect(&silence);
    detector.detect(&tone);

    // Measure: a realistic session shape — held note, silence, re-entry —
    // where every frame runs YIN + SuperFlux and voiced frames build NoteInfo.
    COUNTING.with(|c| c.set(true));
    let mut voiced_frames = 0usize;
    for _ in 0..100 {
        for _ in 0..8 {
            let event = detector.detect(&tone);
            if event.note_info.is_some() {
                voiced_frames += 1;
            }
        }
        detector.detect(&silence);
        detector.detect(&tone);
    }
    COUNTING.with(|c| c.set(false));

    let allocs = ALLOC_COUNT.with(|c| c.get());
    assert!(
        voiced_frames >= 800,
        "expected the loop to actually hit the NoteInfo-building path, got {voiced_frames} voiced frames"
    );
    assert_eq!(
        allocs, 0,
        "analysis path allocated {allocs} times across the detect loop; \
         detect() is documented audio-thread-safe and must never allocate (#245)"
    );
}
