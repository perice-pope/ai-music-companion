import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import PitchDisplay from "./components/PitchDisplay";
import { useAudioStore, type AudioEvent } from "./stores/audioStore";

function App() {
  const [backendResponse, setBackendResponse] = useState<string>("");
  const setEvent = useAudioStore((s) => s.setEvent);
  const setListening = useAudioStore((s) => s.setListening);

  useEffect(() => {
    invoke<string>("ping")
      .then((response) => setBackendResponse(response))
      .catch((err: unknown) =>
        console.error("Failed to invoke ping:", err),
      );
  }, []);

  useEffect(() => {
    // Subscribe to audio events from the Rust backend
    const unlisten = listen<AudioEvent>("audio-event", (event) => {
      setEvent(event.payload);
    });

    setListening(true);

    return () => {
      unlisten.then((fn) => fn());
      setListening(false);
    };
  }, [setEvent, setListening]);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-8 bg-gray-900 text-white">
      <div className="text-center">
        <h1 className="text-4xl font-bold">AI Music Companion</h1>
        <p className="mt-2 text-sm text-gray-600">Phase 0 — Spike</p>
        {backendResponse && (
          <p className="mt-4 text-green-400" data-testid="backend-response">
            Backend says: {backendResponse}
          </p>
        )}
      </div>
      <PitchDisplay />
    </main>
  );
}

export default App;
