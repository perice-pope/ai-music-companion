import { useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";

/**
 * "Play with me" — toggles the follow-me backing band on/off during a session.
 *
 * The band follows the player: it stays silent until the live clock locks onto
 * their pulse, then plays drums + bass + pad in the detected key and tempo. The
 * `playing` state is authoritative from the backend's `accompaniment-status`
 * event (the band can stop on its own at session end), so this button reflects
 * that store value rather than optimistic local state.
 *
 * Only enabled while a session is live (`status === "listening"`) — there's no
 * pulse to follow otherwise.
 */
export default function AccompanimentToggle() {
  const status = usePracticeStore((s) => s.status);
  const playing = usePracticeStore((s) => s.accompanimentPlaying);
  const startAccompaniment = usePracticeStore((s) => s.startAccompaniment);
  const stopAccompaniment = usePracticeStore((s) => s.stopAccompaniment);
  const [busy, setBusy] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  const disabled = status !== "listening" || busy;

  const onClick = async () => {
    setLocalError(null);
    setBusy(true);
    try {
      if (playing) {
        await stopAccompaniment();
      } else {
        await startAccompaniment();
      }
    } catch (err) {
      // Surface a calm, human message; keep the raw error in the console. The
      // live session must never break because the band failed to start/stop.
      console.error("accompaniment toggle failed:", err);
      setLocalError(
        playing
          ? "Couldn't stop the band."
          : "Couldn't start the band — check your audio output device.",
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex items-center gap-2">
      {localError && (
        <span className="text-xs text-red-400" role="alert">
          {localError}
        </span>
      )}
      {playing && (
        <span
          role="status"
          aria-live="polite"
          data-testid="accompaniment-status-chip"
          className="flex items-center gap-1 rounded-full border border-emerald-700 bg-emerald-900/40 px-3 py-1 text-xs font-medium uppercase tracking-wider text-emerald-300"
        >
          <span aria-hidden="true">🎵</span> Band playing
        </span>
      )}
      <button
        type="button"
        onClick={onClick}
        disabled={disabled}
        aria-pressed={playing}
        data-testid="accompaniment-toggle"
        className={`rounded-full px-4 py-1 text-sm font-medium transition-colors
          ${
            disabled
              ? "cursor-not-allowed bg-gray-800 text-gray-500"
              : playing
                ? "bg-emerald-600 text-white hover:bg-emerald-500"
                : "bg-blue-600 text-white hover:bg-blue-500"
          }`}
      >
        {playing ? "Stop band" : "Play with me"}
      </button>
    </div>
  );
}
