import {
  usePracticeStore,
  KEY_ASSERT_CONFIDENCE,
} from "../stores/practiceStore";
import { colorForPitchClass } from "../lib/rvColors";

/** Below this key confidence (0–1), the reading is shown tentatively ("maybe …")
 * rather than asserted. The relative alternative is always offered regardless —
 * it's the honesty cue, most useful exactly when we're unsure. */
const KEY_CONFIDENCE_THRESHOLD = KEY_ASSERT_CONFIDENCE;

/**
 * Live "here's what I hear" strip, shown during a session. Surfaces the app's
 * perception — tempo (and whether it has locked a steady pulse) and the detected
 * key with its honest relative alternative — so the adaptive engine stops being a
 * black box. Driven by the backend `perception` event (~8 Hz).
 *
 * Also carries a quiet speakers tip: Bluetooth output drops the band out while
 * the mic is live (a confirmed gotcha), so built-in/wired is most reliable.
 *
 * Chord surfaces gate on the instrument's polyphonic capability (#392): a
 * singer can't produce a chord, so on mono instruments the chord label and
 * "hearing several notes…" are room noise, never perception. Room listening
 * lifts the gate — there the ROOM is the signal, on any instrument.
 */
export default function PerceptionPanel({
  instrumentPolyphonic,
  roomListening,
}: {
  /** From the active instrument's `InstrumentInfo` (profile attack type). */
  instrumentPolyphonic: boolean;
  /**
   * "Listen to the room" is ON and the jam lane is actually the stage.
   * The session computes this — the raw store flag survives into lesson /
   * explore / score views where the mic is back on the player, and there
   * the gate must hold (the reviewer's two-click repro on #392).
   */
  roomListening: boolean;
}) {
  const status = usePracticeStore((s) => s.status);
  const perception = usePracticeStore((s) => s.perception);
  const keyPinned = usePracticeStore((s) => s.keyPinned);
  const pinnedKey = usePracticeStore((s) => s.pinnedKey);
  const setAccompanimentKey = usePracticeStore((s) => s.setAccompanimentKey);
  const lockAccompanimentKey = usePracticeStore((s) => s.lockAccompanimentKey);
  const clearAccompanimentKey = usePracticeStore(
    (s) => s.clearAccompanimentKey,
  );

  // Only meaningful while a session is actually listening.
  if (status !== "listening") return null;

  const tempo = perception?.tempo_bpm ?? null;
  const locked = perception?.locked ?? false;
  const key = perception?.key ?? null;
  const chordCapable = instrumentPolyphonic || roomListening;
  const chord = chordCapable ? (perception?.chord ?? null) : null;
  const hearingPolyphony = chordCapable
    ? (perception?.hearing_polyphony ?? false)
    : false;

  const tempoText =
    tempo == null
      ? null
      : locked
        ? `${Math.round(tempo)} BPM`
        : `~${Math.round(tempo)} BPM · finding the pulse`;

  const nothingHeard =
    tempoText == null && key == null && chord == null && !hearingPolyphony;

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
          {/* Live chord label (#349 jazz ears): the root wears its RV colour.
              When several notes sound but nothing fits confidently, say so
              honestly instead of guessing a name. */}
          {chord ? (
            <span
              data-testid="perception-chord"
              className="flex items-center gap-1.5 font-semibold text-gray-100"
              title={`chord confidence ${Math.round(chord.confidence * 100)}%`}
            >
              <span
                aria-hidden="true"
                className="inline-block h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: colorForPitchClass(chord.root_pc) }}
              />
              {chord.label}
            </span>
          ) : (
            hearingPolyphony && (
              <span
                data-testid="perception-polyphony"
                className="italic text-gray-400"
              >
                hearing several notes…
              </span>
            )
          )}
          {/* #404: an unsettled reading (key-less material — long-tone
              warm-ups, chromatic wandering) shows the honest search state,
              never a name the evidence no longer supports. A pinned key is
              the user's own choice and always shows. */}
          {key && !keyPinned && key.settled === false ? (
            <span
              data-testid="perception-key-finding"
              className="italic text-gray-400"
            >
              🎵 finding the key…
            </span>
          ) : key ? (
            <span
              data-testid="perception-key"
              className="flex items-center gap-1.5"
              title={`key confidence ${Math.round(key.confidence * 100)}%`}
            >
              🎵{" "}
              {!keyPinned && key.confidence < KEY_CONFIDENCE_THRESHOLD
                ? "maybe "
                : ""}
              {/* When pinned, show the pinned key (what the band actually plays),
                  not the still-changing auto-detected reading. */}
              {keyPinned && pinnedKey ? pinnedKey.name : key.name}
              {keyPinned ? (
                <>
                  <span className="text-emerald-400" aria-hidden="true">
                    🔒
                  </span>
                  <button
                    type="button"
                    data-testid="key-auto"
                    onClick={() => void clearAccompanimentKey()}
                    className="rounded border border-gray-700 px-1.5 py-0.5 text-[10px] text-gray-300 hover:bg-gray-800"
                  >
                    auto
                  </button>
                </>
              ) : (
                <>
                  {key.alternative && (
                    <button
                      type="button"
                      data-testid="key-use-alternative"
                      onClick={() =>
                        void setAccompanimentKey(
                          key.alternative!.tonic,
                          key.alternative!.minor,
                        )
                      }
                      className="rounded border border-gray-700 px-1.5 py-0.5 text-gray-400 hover:bg-gray-800 hover:text-gray-200"
                    >
                      or {key.alternative.name}?
                    </button>
                  )}
                  <button
                    type="button"
                    data-testid="key-lock"
                    title="Lock the band to this key"
                    onClick={() => void lockAccompanimentKey()}
                    className="rounded border border-gray-700 px-1 py-0.5 text-[10px] text-gray-500 hover:bg-gray-800 hover:text-gray-300"
                  >
                    🔒
                  </button>
                </>
              )}
            </span>
          ) : null}
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
