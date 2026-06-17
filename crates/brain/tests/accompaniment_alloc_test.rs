//! Verifies `AccompanimentSynth::render` does not allocate (AC8).
//!
//! `render` runs on the output engine's render thread feeding the realtime
//! callback, so it must never touch the heap. Counting is **per-thread** (a
//! const-initialised thread-local), so libtest's concurrently-running harness
//! thread can't pollute the count — the same pattern proven on the audio
//! engine's no-alloc test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use brain::accompaniment::AccompanimentSynth;
use ears::output_engine::RenderSource;
use groove::ClockState;
use theory::Mode;

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
// `GlobalAlloc` contract); the only added work is a thread-local counter bump.
// It allocates nothing of its own and returns exactly what `System` returns.
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
fn render_does_not_allocate_in_steady_state() {
    let mut synth = AccompanimentSynth::new(44_100);
    synth.set_key(7, Mode::Mixolydian);
    synth.set_clock(ClockState {
        tempo_bpm: Some(120.0),
        swing_ratio: None,
        beat_phase: 0.0,
        confidence: 1.0,
    });

    // Pre-allocate the render buffer OUTSIDE the measured window.
    let mut buf = vec![0.0f32; 1024];
    synth.render(&mut buf); // warm up

    let locked = ClockState {
        tempo_bpm: Some(120.0),
        swing_ratio: None,
        beat_phase: 0.0,
        confidence: 1.0,
    };

    COUNTING.with(|c| c.set(true));
    for _ in 0..1000 {
        // The locked render path...
        synth.render(&mut buf);
        // ...the control path (set_clock)...
        synth.set_clock(locked);
        // ...and the silent early-return path must all be allocation-free.
        synth.set_clock(ClockState::SILENT);
        synth.render(&mut buf);
        synth.set_clock(locked);
    }
    COUNTING.with(|c| c.set(false));

    let allocs = ALLOC_COUNT.with(|c| c.get());
    assert_eq!(
        allocs, 0,
        "AccompanimentSynth::render allocated {allocs} times; it feeds the audio callback and must not"
    );
}
