import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [backendResponse, setBackendResponse] = useState<string>("");

  useEffect(() => {
    invoke<string>("ping")
      .then((response) => setBackendResponse(response))
      .catch((err: unknown) =>
        console.error("Failed to invoke ping:", err),
      );
  }, []);

  return (
    <main className="flex min-h-screen items-center justify-center bg-gray-900 text-white">
      <div className="text-center">
        <h1 className="text-4xl font-bold">AI Music Companion</h1>
        <p className="mt-4 text-gray-400">Your intelligent practice partner</p>
        <p className="mt-2 text-sm text-gray-600">Phase 0 — Spike</p>
        {backendResponse && (
          <p className="mt-4 text-green-400" data-testid="backend-response">
            Backend says: {backendResponse}
          </p>
        )}
      </div>
    </main>
  );
}

export default App;
