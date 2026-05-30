import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import PracticeShell from "./components/PracticeShell";
import { useAudioStore, type AudioEvent } from "./stores/audioStore";
import { usePracticeStore } from "./stores/practiceStore";
import type { PhraseSummary } from "./types/brain";

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
 */
function App() {
  const setEvent = useAudioStore((s) => s.setEvent);
  const pushPhrase = usePracticeStore((s) => s.pushPhrase);
  const setCursorPosition = usePracticeStore((s) => s.setCursorPosition);

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
          }),
        );
      } catch (err: unknown) {
        console.error("Failed to subscribe to phrase-detected:", err);
      }
    })();

    return () => {
      for (const u of unsubs) u();
    };
  }, [setEvent, pushPhrase, setCursorPosition]);

  return <PracticeShell />;
}

export default App;
