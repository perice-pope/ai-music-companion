import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import PracticeShell from "./components/PracticeShell";
import { useAudioStore, type AudioEvent } from "./stores/audioStore";
import { usePracticeStore } from "./stores/practiceStore";
import type { PhraseSummary, ScorePosition, CoachingTip } from "./types/brain";

/**
 * App entry. Subscribes to the backend's live event streams for the whole
 * app lifetime and hands rendering off to `PracticeShell`.
 *
 * Three subscriptions, all *always on* — the backend only emits while a
 * session is active (see `audio_pipeline.rs`), so there's no per-session
 * teardown here:
 *  - `audio-event`   → the live pitch meter.
 *  - `phrase-detected` → completed phrases; triggers coaching tip fetch.
 *  - `score-position-updated` → live cursor updates for score mode.
 *
 * `isListening` on the audio store is driven by `practiceStore`'s
 * session-lifecycle actions, not by whether a listener is registered —
 * otherwise it would always read `true` from the moment the app mounts
 * and `PitchDisplay` would never show its idle state.
 */
function App() {
  const setEvent = useAudioStore((s) => s.setEvent);
  const pushPhrase = usePracticeStore((s) => s.pushPhrase);
  const pushTip = usePracticeStore((s) => s.pushTip);
  const setCursorPosition = usePracticeStore((s) => s.setCursorPosition);
  const status = usePracticeStore((s) => s.status);
  const elapsedSecs = usePracticeStore((s) => s.elapsedSecs);
  const phrases = usePracticeStore((s) => s.phrases);
  const coachingEnabled = usePracticeStore((s) => s.coachingEnabled);

  useEffect(() => {
    const unsubs: Array<() => void> = [];

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
            // Only score-following phrases carry a position; in free play
            // it's absent and the cursor stays put.
            if (payload.score_position) {
              setCursorPosition(payload.score_position);
            }
            // Fetch a coaching tip if enabled and session is active.
            if (coachingEnabled && status === "listening") {
              (async () => {
                try {
                  const tip = await invoke<CoachingTip | null>(
                    "get_coaching_tip",
                    {
                      phrase: payload,
                      session_duration_secs: elapsedSecs,
                      phrases_played: phrases.length,
                    },
                  );
                  if (tip) {
                    pushTip(tip, phrases.length - 1);
                    // Record the tip in the session so it appears in the recap
                    // and can be used to avoid repetition in future tips.
                    try {
                      await invoke("record_coaching_tip", { tip });
                    } catch (err: unknown) {
                      console.error("Failed to record coaching tip:", err);
                    }
                  }
                } catch (err: unknown) {
                  console.error("Failed to get coaching tip:", err);
                }
              })();
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
          await listen<ScorePosition>("score-position-updated", ({ payload }) => {
            setCursorPosition(payload);
          }),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to score-position-updated:", err);
      }
    })();

    return () => {
      for (const u of unsubs) u();
    };
  }, [setEvent, pushPhrase, pushTip, setCursorPosition, status, elapsedSecs, phrases.length, coachingEnabled]);

  return <PracticeShell />;
}

export default App;
