import { usePracticeStore } from "../stores/practiceStore";

/**
 * #421 S1 — The Pocket: the strict Anchor click with a one-bar count-in.
 *
 * The parity floor of the listening metronome. While playing, the pulse
 * BREATHES — one persistent element swelling and dimming on a continuous
 * curve at the beat period (#417 rule 0: nothing blinks, nothing mounts
 * per beat). The backend is authoritative for playing state and reports
 * the CLAMPED tempo it actually clicks, so the label can never lie.
 */
export default function PocketControl() {
  const status = usePracticeStore((s) => s.status);
  const playing = usePracticeStore((s) => s.pocketPlaying);
  const tempo = usePracticeStore((s) => s.pocketTempo);
  const countIn = usePracticeStore((s) => s.pocketCountIn);
  const setPocketTempo = usePracticeStore((s) => s.setPocketTempo);
  const mode = usePracticeStore((s) => s.pocketMode);
  const setPocketMode = usePracticeStore((s) => s.setPocketMode);
  const frozenBpm = usePracticeStore((s) => s.pocketFrozenBpm);
  const liveTempo = usePracticeStore((s) => s.perception?.tempo_bpm ?? null);
  const setPocketCountIn = usePracticeStore((s) => s.setPocketCountIn);
  const startPocket = usePracticeStore((s) => s.startPocket);
  const stopPocket = usePracticeStore((s) => s.stopPocket);

  const inSession = status === "listening";
  const beatSecs = 60 / Math.max(tempo, 1);

  return (
    <div className="flex items-center gap-2" data-testid="pocket-control">
      {playing ? (
        <>
          <span
            data-testid="pocket-pulse"
            className="inline-block h-3 w-3 rounded-full bg-emerald-400"
            style={{
              animation: `pocket-breathe ${beatSecs}s ease-in-out infinite`,
            }}
            aria-hidden="true"
          />
          <span className="text-sm text-emerald-300" data-testid="pocket-tempo">
            {Math.round(tempo)} BPM
          </span>
          {mode === "follow" && (
            <span
              className="text-xs text-emerald-400/80"
              data-testid="pocket-following"
            >
              following you
            </span>
          )}
          {mode === "handoff" && frozenBpm !== null && (
            <span
              className="text-xs text-emerald-400/80"
              data-testid="pocket-drift"
            >
              {(() => {
                // #421 S2: the honest drift line — frozen vs live, in
                // place, never blinking (rule 0).
                if (liveTempo === null) {
                  return `holding your ${Math.round(frozenBpm)}`;
                }
                const drift = Math.round(liveTempo - frozenBpm);
                if (Math.abs(drift) < 2) {
                  return `holding your ${Math.round(frozenBpm)} — in the pocket`;
                }
                return `holding your ${Math.round(frozenBpm)} — you're ${
                  drift > 0 ? `+${drift}` : drift
                } BPM ${drift > 0 ? "ahead" : "behind"}`;
              })()}
            </span>
          )}
          <button
            type="button"
            data-testid="pocket-stop"
            onClick={() => void stopPocket()}
            className="rounded-md border border-emerald-700 px-2 py-1 text-sm text-emerald-200 hover:bg-emerald-900/40"
          >
            Stop click
          </button>
        </>
      ) : (
        <>
          <input
            type="number"
            min={40}
            max={220}
            value={tempo}
            data-testid="pocket-tempo-input"
            disabled={!inSession}
            onChange={(e) => setPocketTempo(Number(e.target.value))}
            onBlur={(e) => {
              // A cleared field must not park at "0" — settle into range.
              const n = Number(e.target.value);
              setPocketTempo(
                Number.isFinite(n) && n > 0
                  ? Math.min(Math.max(n, 40), 220)
                  : 90,
              );
            }}
            className="w-16 rounded-md border border-gray-700 bg-gray-900 px-2 py-1 text-sm text-gray-200 disabled:opacity-40"
            aria-label="Click tempo (BPM)"
          />
          <label className="flex items-center gap-1 text-xs text-gray-400">
            <input
              type="checkbox"
              data-testid="pocket-count-in"
              checked={countIn}
              disabled={!inSession}
              onChange={(e) => setPocketCountIn(e.target.checked)}
            />
            count-in
          </label>
          {(["anchor", "follow", "handoff"] as const).map((m) => (
            <button
              key={m}
              type="button"
              data-testid={`pocket-mode-${m}`}
              aria-pressed={mode === m}
              disabled={!inSession}
              onClick={() => setPocketMode(m)}
              className={`rounded-md px-2 py-1 text-xs ${
                mode === m
                  ? "bg-gray-600 font-semibold text-white"
                  : "border border-gray-700 text-gray-300 hover:bg-gray-800"
              } disabled:opacity-40`}
            >
              {m}
            </button>
          ))}
          <button
            type="button"
            data-testid="pocket-start"
            disabled={!inSession}
            onClick={() => void startPocket()}
            className="rounded-md border border-gray-700 px-2 py-1 text-sm text-gray-200 hover:bg-gray-800 disabled:opacity-40"
          >
            ▔▏Click
          </button>
        </>
      )}
    </div>
  );
}
