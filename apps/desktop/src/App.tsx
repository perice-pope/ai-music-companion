import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import PracticeShell from "./components/PracticeShell";
import UpdatePill from "./components/UpdatePill";
import { useConnectionsStore } from "./stores/connectionsStore";
import { useUpdateStore } from "./stores/updateStore";
import { useAudioStore, type AudioEvent } from "./stores/audioStore";
import { usePracticeStore } from "./stores/practiceStore";
import type { NoteVerdictKind } from "./stores/practiceStore";
import type {
  AccompanimentStatus,
  PerceptionSnapshot,
  PhraseSummary,
  ScorePosition,
} from "./types/brain";
import {
  freshReceiptState,
  receiptBreadcrumb,
  sendBreadcrumb,
} from "./lib/positionBreadcrumbs";

/**
 * App entry. Subscribes to the backend's live event streams for the whole
 * app lifetime and hands rendering off to `PracticeShell`.
 *
 * Two subscriptions, both *always on* — the backend only emits while a
 * session is active (see `audio_pipeline.rs`), so there's no per-session
 * teardown here:
 *  - `audio-event`   → the live pitch meter.
 *  - `phrase-detected` → completed phrases; in score mode each carries a
 *    `score_position` that advances the on-screen cursor.
 *
 * `isListening` on the audio store is driven by `practiceStore`'s
 * session-lifecycle actions, not by whether a listener is registered —
 * otherwise it would always read `true` from the moment the app mounts
 * and `PitchDisplay` would never show its idle state.
 *
 * Routing (including the Connections & Privacy panel that discloses every
 * networked feature — see
 * `docs/architecture/offline-first-and-network-transparency.md`) lives in
 * `PracticeShell`'s `screen` state machine. The subscriptions here are
 * local-only; nothing in this always-on shell touches the network.
 */
function App() {
  const setEvent = useAudioStore((s) => s.setEvent);
  const pushPhrase = usePracticeStore((s) => s.pushPhrase);
  const requestCoachingTip = usePracticeStore((s) => s.requestCoachingTip);
  const requestReveal = usePracticeStore((s) => s.requestReveal);
  const setCursorPosition = usePracticeStore((s) => s.setCursorPosition);
  const setAccompanimentPlaying = usePracticeStore(
    (s) => s.setAccompanimentPlaying,
  );
  const setPerception = usePracticeStore((s) => s.setPerception);
  const [storageDegraded, setStorageDegraded] = useState(false);
  const autoUpdateCheckEnabled = useConnectionsStore(
    (s) => s.autoUpdateCheckEnabled,
  );
  const checkForUpdate = useUpdateStore((s) => s.checkForUpdate);

  useEffect(() => {
    // #58: the pill's heartbeat — STRICTLY opt-in. With the toggle off
    // (the default) this effect does nothing at all: the shipped promise
    // is "no update request on launch or in the background."
    if (!autoUpdateCheckEnabled) {
      return;
    }
    void checkForUpdate();
    const FOUR_HOURS_MS = 4 * 60 * 60 * 1000;
    const interval = setInterval(() => void checkForUpdate(), FOUR_HOURS_MS);
    return () => clearInterval(interval);
  }, [autoUpdateCheckEnabled, checkForUpdate]);

  useEffect(() => {
    // One-shot capability check at mount. If on-disk persistence fell back to
    // in-memory at startup (disk permissions, sandbox, full disk), warn the
    // musician calmly rather than silently dropping their history (#137).
    void (async () => {
      try {
        const caps = await invoke<{
          coaching_available: boolean;
          storage_degraded: boolean;
        }>("get_app_capabilities");
        setStorageDegraded(caps.storage_degraded);
      } catch (err: unknown) {
        console.error("Failed to fetch app capabilities:", err);
      }
    })();
  }, []);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    // #354 diagnostics: app-lifetime receipt counter for the position
    // stream (the backend's counterpart is per session — its 'first'
    // lines mark session starts in the shared log).
    const receiptState = freshReceiptState();

    (async () => {
      try {
        unsubs.push(
          await listen<AudioEvent>("audio-event", ({ payload }) => {
            setEvent(payload);
          }),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to audio-event:", err);
      }

      try {
        unsubs.push(
          await listen<PhraseSummary>("phrase-detected", ({ payload }) => {
            pushPhrase(payload);
            // Live coaching loop: ask the backend for a tip on this phrase.
            // Self-gated on the user's `coachingEnabled` opt-in — when off it
            // fires no IPC at all. Fire-and-forget; the live session must never
            // block on (or break from) a coaching round-trip.
            void requestCoachingTip(payload);
            // Ambient reveal loop: surface a real-world music connection for the
            // current key/mode (curated + offline). Self-gated to a live session
            // with a detected key; fire-and-forget, like coaching.
            void requestReveal(payload);
            // Only score-following phrases carry a position; in free play
            // it's absent and the cursor stays put.
            if (payload.score_position) {
              setCursorPosition(payload.score_position);
            }
          }),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to phrase-detected:", err);
      }

      try {
        unsubs.push(
          // Fine-grained (~10 Hz) cursor updates between phrase boundaries.
          // Only emitted in score mode, so no free-play guard is needed.
          await listen<ScorePosition>(
            "score-position-updated",
            ({ payload }) => {
              // #354 diagnostics: mirror the backend's emission log with a
              // receipt log, so a tester's capture shows whether positions
              // crossed the IPC boundary. Fire-and-forget, local-only.
              const line = receiptBreadcrumb(
                receiptState,
                payload.measure_number,
              );
              if (line) sendBreadcrumb(line);
              setCursorPosition(payload);
            },
          ),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to score-position-updated:", err);
      }

      try {
        unsubs.push(
          // Live hit/near/missed verdicts (#337 S2) — boundary cadence,
          // score sessions only.
          await listen<{ verdict: NoteVerdictKind }>(
            "note-verdict",
            ({ payload }) => {
              usePracticeStore.getState().recordNoteVerdict(payload.verdict);
            },
          ),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to note-verdict:", err);
      }

      try {
        unsubs.push(
          // Follow-me band on/off. The backend is authoritative — it may stop
          // the band on its own (session end), so the toggle reflects this.
          await listen<AccompanimentStatus>(
            "accompaniment-status",
            ({ payload }) => {
              setAccompanimentPlaying(payload.playing);
            },
          ),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to accompaniment-status:", err);
      }

      try {
        unsubs.push(
          // Live "what the app hears" (~8 Hz): tempo / feel / key + confidence.
          await listen<PerceptionSnapshot>("perception", ({ payload }) => {
            setPerception(payload);
          }),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to perception:", err);
      }
    })();

    return () => {
      for (const u of unsubs) u();
    };
  }, [
    setEvent,
    pushPhrase,
    requestCoachingTip,
    requestReveal,
    setCursorPosition,
    setAccompanimentPlaying,
    setPerception,
  ]);

  return (
    <>
      {storageDegraded && (
        <div
          role="status"
          className="fixed top-0 left-0 right-0 z-50 border-b border-yellow-700 bg-yellow-900 px-4 py-3 text-sm text-yellow-100"
        >
          ⚠️ Session storage is unavailable — your practice history won't be
          saved this session.
        </div>
      )}
      <div className={storageDegraded ? "pt-12" : ""}>
        <PracticeShell />
      </div>
      <UpdatePill />
    </>
  );
}

export default App;
