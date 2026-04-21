import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import PracticeShell from "./components/PracticeShell";
import { useAudioStore, type AudioEvent } from "./stores/audioStore";

/**
 * App entry. Wires the pre-existing `audio-event` subscription from
 * Phase 0 (so `PitchDisplay` keeps working inside `PracticeSession`)
 * and hands rendering off to `PracticeShell`, which owns screen
 * routing via the practice store.
 *
 * PR 1 note: the Phase-0 `ping` handshake banner has moved off the
 * visible render path now that we're on the free-play shell. Real
 * wiring for `phrase-detected` / `coaching-tip` events lands in PR 2.
 */
function App() {
  const setEvent = useAudioStore((s) => s.setEvent);
  const setListening = useAudioStore((s) => s.setListening);

  useEffect(() => {
    let isMounted = true;
    let unsubscribe: (() => void) | null = null;

    (async () => {
      try {
        unsubscribe = await listen<AudioEvent>("audio-event", ({ payload }) => {
          setEvent(payload);
        });
        if (isMounted) setListening(true);
      } catch (err: unknown) {
        console.error("Failed to subscribe to audio-event:", err);
        if (isMounted) setListening(false);
      }
    })();

    return () => {
      isMounted = false;
      unsubscribe?.();
      setListening(false);
    };
  }, [setEvent, setListening]);

  return <PracticeShell />;
}

export default App;
