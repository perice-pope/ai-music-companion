//! Verifies the output ring buffer does not allocate on the hot path.
//!
//! The audio-thread rule (CLAUDE.md) is: NEVER allocate in the audio callback.
//! Every device-format callback drains via one of `OutputConsumer::fill` /
//! `fill_i16` / `fill_u16` (all share the same `fill_with` core); the render
//! thread only calls `OutputProducer::push_samples`. Unlike the
//! algorithmic-purity proxy used elsewhere, this test installs a global
//! allocator that counts allocations and asserts that the steady-state
//! drain/push path — for all three sample formats — performs **zero** of them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ears::output_engine::create_output_buffer;

/// A pass-through allocator that counts allocations while `COUNTING` is on.
struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

// SAFETY: `CountingAlloc` forwards every call verbatim to the `System`
// allocator (which upholds the `GlobalAlloc` contract) and adds only a counter
// increment on an atomic. It allocates no memory of its own and returns exactly
// what `System` returns, so all pointer/layout invariants are preserved.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn fill_and_push_do_not_allocate_in_steady_state() {
    let (mut producer, mut consumer) = create_output_buffer(4096);

    // Pre-allocate everything OUTSIDE the measured window: the push block and
    // one output scratch per device format (f32 / i16 / u16).
    let block = vec![0.25f32; 512];
    let mut out_f32 = vec![0.0f32; 480]; // 240 frames x 2 channels
    let mut out_i16 = vec![0i16; 480];
    let mut out_u16 = vec![0u16; 480];

    // Warm up: prime the ring and exercise every drain path once.
    producer.push_samples(&block);
    consumer.fill(&mut out_f32, 2);
    consumer.fill_i16(&mut out_i16, 2);
    consumer.fill_u16(&mut out_u16, 2);

    // Measure: a realistic loop of pushes and drains across all three formats.
    // None of these may allocate — the ring storage and all scratch buffers
    // already exist.
    COUNTING.store(true, Ordering::SeqCst);
    for _ in 0..1000 {
        producer.push_samples(&block);
        consumer.fill(&mut out_f32, 2);
        consumer.fill_i16(&mut out_i16, 2);
        consumer.fill_u16(&mut out_u16, 2);
    }
    COUNTING.store(false, Ordering::SeqCst);

    let allocs = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(
        allocs, 0,
        "output ring hot path allocated {allocs} times; the audio callback must never allocate"
    );
}
