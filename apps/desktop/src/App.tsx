import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import PracticeShell from "./components/PracticeShell";
import { useAudioStore, type AudioEvent } from "./stores/audioStore";

/**
 * App entry. Subscribes to the backend's `audio-event` stream for the
 * whole app lifetime and hands rendering off to `PracticeShell`.
 *
 * Note the subscription is *always on* — the backend only emits events
 * while a session is active (see `audio_pipeline.rs`), so there's no
 * per-session teardown to do here. `isListening` on the audio store is
 * driven by `practiceStore`'s session-lifecycle actions, not by whether
 * the listener is registered — otherwise it would always read `true`
 * from the moment the app mounts and `PitchDisplay` would never show
 * its idle state.
 */
function App() {
  const setEvent = useAudioStore((s) => s.setEvent);

  useEffect(() => {
    let unsubscribe: (() => void) | null = null;

    (async () => {
      try {
        unsubscribe = await listen<AudioEvent>("audio-event", ({ payload }) => {
          setEvent(payload);
        });
      } catch (err: unknown) {
        console.error("Failed to subscribe to audio-event:", err);
      }
    })();

    return () => {
      unsubscribe?.();
    };
  }, [setEvent]);

  return <PracticeShell />;
}

export default App;
