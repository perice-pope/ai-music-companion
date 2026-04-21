import { useEffect } from "react";
import { usePracticeStore } from "../stores/practiceStore";

/** Format elapsed seconds as `MM:SS` (pads both parts, overflow OK). */
export function formatMmSs(elapsedSecs: number): string {
  // Defensive: negative values collapse to 00:00 rather than printing
  // something confusing like `-1:-1`.
  const secs = Math.max(0, Math.floor(elapsedSecs));
  const mm = Math.floor(secs / 60)
    .toString()
    .padStart(2, "0");
  const ss = (secs % 60).toString().padStart(2, "0");
  return `${mm}:${ss}`;
}

/**
 * Top-right session timer.
 *
 * Drives a 1Hz setInterval that calls the store's `tick()` while the
 * session is active, and cleans it up the moment status moves off
 * `listening`. The store holds `elapsedSecs` so the timer renders at
 * most 1Hz regardless of surrounding component re-renders.
 */
export default function SessionTimer() {
  const elapsedSecs = usePracticeStore((s) => s.elapsedSecs);
  const status = usePracticeStore((s) => s.status);
  const tick = usePracticeStore((s) => s.tick);

  useEffect(() => {
    if (status !== "listening") return;
    // First tick fires immediately so the UI shows a correct value
    // without waiting a full second after mount.
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [status, tick]);

  return (
    <span
      className="font-mono text-sm text-gray-300"
      data-testid="session-timer"
      aria-label="Session timer"
    >
      {formatMmSs(elapsedSecs)}
    </span>
  );
}
