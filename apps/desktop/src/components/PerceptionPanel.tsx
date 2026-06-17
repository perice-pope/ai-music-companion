import { usePracticeStore } from "../stores/practiceStore";

/** Below this key confidence (0–1), the reading is shown tentatively ("maybe …")
 * rather than asserted. The relative alternative is always offered regardless —
 * it's the honesty cue, most useful exactly when we're unsure. */
const KEY_CONFIDENCE_THRESHOLD = 0.55;

/**
 * Live "here's what I hear" strip, shown during a session. Surfaces the app's
 * perception — tempo (and whether it has locked a steady pulse) and the detected
 * key with its honest relative alternative — so the adaptive engine stops being a
 * black box. Driven by the backend `perception` event (~8 Hz).
 *
 * Also carries a quiet speakers tip: Bluetooth output drops the band out while
 * the mic is live (a confirmed gotcha), so built-in/wired is most reliable.
 */
export default function PerceptionPanel() {
  const status = usePracticeStore((s) => s.status);
  const perception = usePracticeStore((s) => s.perception);

  // Only meaningful while a session is actually listening.
  if (status !== "listening") return null;

  const tempo = perception?.tempo_bpm ?? null;
  const locked = perception?.locked ?? false;
  const key = perception?.key ?? null;

  const tempoText =
    tempo == null
      ? null
      : locked
        ? `${Math.round(tempo)} BPM`
        : `~${Math.round(tempo)} BPM · finding the pulse`;

  const nothingHeard = tempoText == null && key == null;

  return (
    <div
      data-testid="perception-panel"
      role="status"
      aria-live="polite"
      className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-gray-800 bg-gray-900/60 px-4 py-1.5 text-xs text-gray-300"
    >
      <span className="font-medium uppercase tracking-wider text-gray-500">
        I hear
      </span>

      {nothingHeard ? (
        <span data-testid="perception-listening" className="text-gray-400">
          🎧 listening…
        </span>
      ) : (
        <>
          {tempoText && <span data-testid="perception-tempo">{tempoText}</span>}
          {key && (
            <span
              data-testid="perception-key"
              title={`key confidence ${Math.round(key.confidence * 100)}%`}
            >
              🎵 {key.confidence < KEY_CONFIDENCE_THRESHOLD ? "maybe " : ""}
              {key.name}
              {key.alternative && (
                <span className="text-gray-500"> — or {key.alternative}?</span>
              )}
            </span>
          )}
        </>
      )}

      <span
        data-testid="perception-output-tip"
        className="ml-auto text-[10px] text-gray-600"
      >
        Tip: use built-in/wired speakers — Bluetooth can drop the band out while
        your mic is live.
      </span>
    </div>
  );
}
