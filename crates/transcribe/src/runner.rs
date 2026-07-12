//! #349 T3b — the polyphony runner: a [`PolyEngine`] on its own thread.
//!
//! One basic-pitch hop costs ~38 ms; the session's analysis loop runs at
//! ~43 Hz with a <25 ms budget, so inference can NEVER run inline. The
//! runner owns a dedicated thread: the audio worker `feed()`s it mono
//! windows through a bounded channel (non-blocking — a stalled consumer
//! drops audio honestly instead of stalling the loop), the thread runs the
//! engine, and consumers read the CURRENTLY SOUNDING notes from a shared
//! snapshot.
//!
//! First consumer (this slice): **voicing-true bass** — the lowest note
//! actually sounding is the bass BY DEFINITION, replacing perception's
//! YIN register heuristic for slash labels. The ~1–2 s engine latency is
//! fine there: the chord tracker refreshes a ringing label's slash in
//! place, so a late true bass upgrades "C" → "C/E" while the chord still
//! rings (lane and chart refresh in place too — built for this).
//!
//! Kill-switch (T3 AC4): [`PolyRunner::spawn`] is fallible exactly like
//! the engine's constructor — no ONNX Runtime → calm `Err`, the session
//! runs without polyphony, nothing else changes.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::error::TranscribeError;
use crate::stream::{PolyEngine, PolyNote, StreamingBasicPitch};

/// Bounded feed depth: ~1.5 s of 1024-sample windows. If the inference
/// thread falls behind, the oldest audio is dropped at the SENDER (feed
/// returns immediately) — the analysis loop never blocks on the model.
const FEED_DEPTH: usize = 64;

/// A note keeps counting as "sounding" this long past its detected offset —
/// the engine's own hop cadence means offsets trail reality by up to a hop.
const RING_GRACE_SECS: f64 = 0.5;

/// One fed chunk: mono samples + their sample rate.
type Chunk = (Vec<f32>, u32);

/// What the runner shares back: notes believed to be sounding now.
#[derive(Debug, Default)]
struct PolyShared {
    /// Active notes (pruned as the stream clock passes their offsets).
    active: Vec<PolyNote>,
    /// The engine's stream clock: seconds of audio consumed so far.
    stream_secs: f64,
}

/// A [`PolyEngine`] on its own thread, feeding a currently-sounding
/// snapshot. Generic over the engine so the threading contract is testable
/// without ONNX (see the mock-engine tests).
pub struct PolyRunner {
    tx: SyncSender<Chunk>,
    shared: Arc<Mutex<PolyShared>>,
    shutdown: Arc<AtomicBool>,
    /// Samples dropped at the sender on backpressure — the thread converts
    /// them to SILENCE so the engine's stream clock stays aligned with the
    /// session clock (a silent drop would shift every later timestamp).
    dropped_samples: Arc<AtomicU64>,
    /// The stream's sample rate, learned from the first feed (sessions
    /// don't change rate mid-stream).
    sr: Arc<AtomicU32>,
    thread: Option<JoinHandle<()>>,
}

impl PolyRunner {
    /// Spawn with the real basic-pitch engine. Errors calmly when the
    /// runtime is unavailable — the caller ships without polyphony.
    pub fn spawn() -> Result<Self, TranscribeError> {
        let engine = StreamingBasicPitch::new()?;
        Ok(Self::spawn_with(Box::new(engine)))
    }

    /// Spawn with ANY engine (the seam again — and the test hook).
    pub fn spawn_with(engine: Box<dyn PolyEngine>) -> Self {
        let (tx, rx) = sync_channel::<Chunk>(FEED_DEPTH);
        let shared = Arc::new(Mutex::new(PolyShared::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let dropped_samples = Arc::new(AtomicU64::new(0));
        let sr = Arc::new(AtomicU32::new(0));
        let thread = std::thread::Builder::new()
            .name("poly-runner".into())
            .spawn({
                let shared = Arc::clone(&shared);
                let shutdown = Arc::clone(&shutdown);
                let dropped = Arc::clone(&dropped_samples);
                let sr = Arc::clone(&sr);
                move || run(engine, rx, shared, shutdown, dropped, sr)
            })
            .expect("spawning the poly thread");
        Self {
            tx,
            shared,
            shutdown: Arc::clone(&shutdown),
            dropped_samples,
            sr,
            thread: Some(thread),
        }
    }

    /// Push a window of mono audio. NEVER blocks: a full queue records the
    /// window as DROPPED, and the thread replays that much silence so the
    /// engine's stream clock never drifts against the session clock.
    pub fn feed(&self, samples: &[f32], sample_rate: u32) {
        self.sr.store(sample_rate, Ordering::Relaxed);
        match self.tx.try_send((samples.to_vec(), sample_rate)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped_samples
                    .fetch_add(samples.len() as u64, Ordering::Relaxed);
            }
        }
    }

    /// The lowest note sounding at `stream_secs` (the session's audio
    /// clock — the same one the fed samples advance). `None` = the engine
    /// hasn't heard simultaneous voicing there, or its picture has aged
    /// out; callers keep their own fallback (YIN, for the slash heuristic).
    pub fn sounding_bass(&self, stream_secs: f64) -> Option<u8> {
        let shared = self.shared.lock().ok()?;
        shared
            .active
            .iter()
            .filter(|n| n.on_secs <= stream_secs && n.off_secs + RING_GRACE_SECS >= stream_secs)
            .map(|n| n.midi)
            .min()
    }

    /// Graceful stop; also happens on drop.
    pub fn stop(mut self) {
        self.shutdown_and_join();
    }

    fn shutdown_and_join(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Unblock a parked recv by dropping our sender clone? The receiver
        // wakes on disconnect only when ALL senders drop — which happens
        // when `self` drops. Nudge with an empty chunk instead.
        let _ = self.tx.try_send((Vec::new(), 22_050));
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for PolyRunner {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

fn run(
    mut engine: Box<dyn PolyEngine>,
    rx: Receiver<Chunk>,
    shared: Arc<Mutex<PolyShared>>,
    shutdown: Arc<AtomicBool>,
    dropped_samples: Arc<AtomicU64>,
    stream_sr: Arc<AtomicU32>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        let Ok((samples, sr)) = rx.recv() else {
            break; // all senders gone
        };
        // Replay any backpressure drops as silence FIRST, so this chunk
        // lands at its true stream position.
        let owed = dropped_samples.swap(0, Ordering::Relaxed);
        let mut secs = samples.len() as f64 / f64::from(sr.max(1));
        if owed > 0 {
            let gap_sr = stream_sr.load(Ordering::Relaxed).max(1);
            let silence = vec![0.0f32; owed as usize];
            engine.feed(&silence, gap_sr);
            secs += owed as f64 / f64::from(gap_sr);
        }
        engine.feed(&samples, sr);
        // poll() no-ops until a hop is ready; an inference error is calm —
        // the next hop may succeed, and the snapshot just goes quiet.
        let notes = engine.poll().unwrap_or_default();
        if let Ok(mut s) = shared.lock() {
            s.stream_secs += secs;
            let now = s.stream_secs;
            s.active.extend(notes);
            s.active
                .retain(|n| n.off_secs + RING_GRACE_SECS >= now - RING_GRACE_SECS);
            // Hard cap: a pathological engine can't grow the snapshot
            // unboundedly (~an orchestra's worth is plenty).
            if s.active.len() > 256 {
                let overflow = s.active.len() - 256;
                s.active.drain(..overflow);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    /// A scripted engine: no ONNX, returns queued notes on the nth poll.
    struct MockEngine {
        fed: Arc<AtomicUsize>,
        emit_on_feed: usize,
        notes: Vec<PolyNote>,
        block: Option<Duration>,
    }

    impl PolyEngine for MockEngine {
        fn feed(&mut self, _samples: &[f32], _sample_rate: u32) {
            self.fed.fetch_add(1, Ordering::SeqCst);
            if let Some(d) = self.block {
                std::thread::sleep(d);
            }
        }
        fn poll(&mut self) -> Result<Vec<PolyNote>, TranscribeError> {
            if self.fed.load(Ordering::SeqCst) >= self.emit_on_feed {
                Ok(std::mem::take(&mut self.notes))
            } else {
                Ok(Vec::new())
            }
        }
        fn finish(&mut self) -> Result<Vec<PolyNote>, TranscribeError> {
            Ok(Vec::new())
        }
    }

    fn note(midi: u8, on: f64, off: f64) -> PolyNote {
        PolyNote {
            midi,
            on_secs: on,
            off_secs: off,
            amplitude: 0.8,
        }
    }

    /// THE threading contract: feed() never blocks, even against an engine
    /// slower than the incoming audio — excess windows drop at the sender.
    /// Fails if the channel becomes unbounded or feed starts waiting.
    #[test]
    fn feed_never_blocks_on_a_slow_engine() {
        let fed = Arc::new(AtomicUsize::new(0));
        let runner = PolyRunner::spawn_with(Box::new(MockEngine {
            fed: Arc::clone(&fed),
            emit_on_feed: usize::MAX,
            notes: Vec::new(),
            block: Some(Duration::from_millis(50)),
        }));
        let chunk = vec![0.0f32; 1024];
        let start = Instant::now();
        for _ in 0..500 {
            runner.feed(&chunk, 22_050);
        }
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "500 feeds against a 50ms/feed engine must not serialize: {:?}",
            start.elapsed()
        );
        runner.stop();
    }

    /// The sounding snapshot answers with the LOWEST active note at the
    /// stream clock, ages notes out past the ring grace, and stays quiet
    /// before anything was heard.
    #[test]
    fn sounding_bass_is_the_lowest_active_note() {
        let fed = Arc::new(AtomicUsize::new(0));
        let runner = PolyRunner::spawn_with(Box::new(MockEngine {
            fed: Arc::clone(&fed),
            emit_on_feed: 1,
            // A C/E voicing: E2 below two upper tones, ringing 0.5–3 s.
            notes: vec![note(64, 0.5, 3.0), note(40, 0.5, 3.0), note(67, 0.5, 3.0)],
            block: None,
        }));
        assert_eq!(runner.sounding_bass(1.0), None, "nothing heard yet");
        // One second of audio → the mock emits on its first poll.
        let chunk = vec![0.0f32; 22_050];
        runner.feed(&chunk, 22_050);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if runner.sounding_bass(1.0) == Some(40) {
                break;
            }
            assert!(Instant::now() < deadline, "bass never surfaced");
            std::thread::sleep(Duration::from_millis(10));
        }
        // Before the notes began and long after they aged out: quiet.
        assert_eq!(runner.sounding_bass(0.1), None);
        assert_eq!(runner.sounding_bass(30.0), None);
        runner.stop();
    }

    /// Backpressure drops are replayed as SILENCE: the engine's stream
    /// clock stays aligned with the session clock, so a note heard AFTER a
    /// drop still carries its true timestamp. Fails if drops silently
    /// shift the stream (every later slash would query the wrong instant).
    #[test]
    fn drops_replay_as_silence_and_keep_the_clock() {
        struct CountingEngine {
            total_samples: Arc<AtomicUsize>,
        }
        impl PolyEngine for CountingEngine {
            fn feed(&mut self, samples: &[f32], _sr: u32) {
                self.total_samples
                    .fetch_add(samples.len(), Ordering::SeqCst);
                // Slow enough that a burst overflows the queue.
                std::thread::sleep(Duration::from_millis(3));
            }
            fn poll(&mut self) -> Result<Vec<PolyNote>, TranscribeError> {
                Ok(Vec::new())
            }
            fn finish(&mut self) -> Result<Vec<PolyNote>, TranscribeError> {
                Ok(Vec::new())
            }
        }
        let total = Arc::new(AtomicUsize::new(0));
        let runner = PolyRunner::spawn_with(Box::new(CountingEngine {
            total_samples: Arc::clone(&total),
        }));
        // Burst-feed 300 chunks — far over FEED_DEPTH, so many drop.
        let chunk = vec![0.0f32; 1024];
        for _ in 0..300 {
            runner.feed(&chunk, 22_050);
        }
        // Give the thread time to drain the queue + replay owed silence.
        let want = 300 * 1024;
        let deadline = Instant::now() + Duration::from_secs(5);
        while total.load(Ordering::SeqCst) < want {
            assert!(
                Instant::now() < deadline,
                "engine received {} of {want} samples — drops were not
                 replayed as silence",
                total.load(Ordering::SeqCst)
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        runner.stop();
    }

    /// stop() (and drop) join cleanly even mid-stream — no hang, no panic.
    #[test]
    fn stop_joins_cleanly() {
        let fed = Arc::new(AtomicUsize::new(0));
        let runner = PolyRunner::spawn_with(Box::new(MockEngine {
            fed,
            emit_on_feed: usize::MAX,
            notes: Vec::new(),
            block: Some(Duration::from_millis(5)),
        }));
        runner.feed(&[0.0; 512], 22_050);
        let start = Instant::now();
        runner.stop();
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
