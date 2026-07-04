import { useEffect, useRef, useState } from "react";
import type { ScorePosition } from "../types/brain";

/**
 * Minimal slice of the OpenSheetMusicDisplay surface we depend on. Typed
 * locally (rather than importing OSMD's types) so the component is trivial
 * to drive with a fake in tests — OSMD's real `render()` needs a layout
 * engine that jsdom doesn't have, so unit tests inject a double here.
 */
export interface OsmdLike {
  load(xml: string): Promise<unknown>;
  render(): void;
  clear?(): void;
  cursor: {
    show(): void;
    hide(): void;
    reset(): void;
    next(): void;
    readonly iterator?: { readonly currentMeasureIndex?: number };
  };
}

/** Factory for an OSMD-like instance bound to a container element. */
export type OsmdFactory = (container: HTMLElement) => OsmdLike;

/**
 * Build the default factory: lazy-loads the (heavy) opensheetmusicdisplay
 * bundle so it isn't in the initial app chunk, then constructs a real
 * instance. OSMD ships as UMD, so the constructor hides under `.default`.
 * `ambient` renders the notation in a light ink at a smaller zoom for the
 * transparent, sits-in-the-background treatment (#278).
 */
const makeDefaultFactory = (ambient: boolean): OsmdFactory => (container) => {
  // The dynamic import is resolved lazily inside `load` on first use; we
  // return a thin async proxy so construction itself stays synchronous and
  // the effect can hold a stable handle.
  let inner: OsmdLike | null = null;
  let ctorPromise: Promise<OsmdLike> | null = null;

  const ensure = async (): Promise<OsmdLike> => {
    if (inner) return inner;
    if (!ctorPromise) {
      ctorPromise = import("opensheetmusicdisplay").then((mod) => {
        const Ctor =
          (mod as { OpenSheetMusicDisplay?: unknown }).OpenSheetMusicDisplay ??
          (mod as { default?: { OpenSheetMusicDisplay?: unknown } }).default
            ?.OpenSheetMusicDisplay;
        const OSMD = Ctor as new (
          el: HTMLElement,
          opts: Record<string, unknown>,
        ) => OsmdLike;
        inner = new OSMD(container, {
          autoResize: true,
          backend: "svg",
          drawingParameters: "compact",
          // Ambient (#278): light ink so the staff reads on the app's dark
          // background — the SVG itself is transparent; the old white "page"
          // was only ever the container's background.
          ...(ambient ? { defaultColorMusic: "#E2E8F0" } : {}),
        });
        if (ambient) {
          // Smaller notation for the ambient treatment. OSMD exposes zoom as
          // a property, applied at the next render().
          (inner as unknown as { Zoom?: number }).Zoom = 0.75;
        }
        return inner;
      });
    }
    return ctorPromise;
  };

  return {
    async load(xml: string) {
      const osmd = await ensure();
      return osmd.load(xml);
    },
    render() {
      inner?.render();
    },
    clear() {
      inner?.clear?.();
    },
    get cursor() {
      // Before construction completes there's no cursor; callers guard on
      // load completion, so this only fires after `inner` is set.
      return (
        inner?.cursor ?? {
          show() {},
          hide() {},
          reset() {},
          next() {},
        }
      );
    },
  };
};

/** Stable factory instances so effect deps don't churn between renders. */
const pageFactory = makeDefaultFactory(false);
const ambientFactory = makeDefaultFactory(true);

export interface ScoreViewProps {
  /** Raw MusicXML to render. Nothing renders while this is null. */
  musicXml: string | null;
  /**
   * Where the player currently is, from `phrase-detected`'s
   * `score_position`. The cursor advances to this measure. `null` parks
   * the cursor at the start.
   */
  cursorPosition: ScorePosition | null;
  /**
   * Visual treatment (#278): `"page"` is the classic white sheet; `"ambient"`
   * drops the page entirely — smaller, light-ink notation on a transparent
   * background so the music sits IN the app instead of on a page over it.
   */
  variant?: "page" | "ambient";
  /** Test seam — defaults to the real lazy-loaded OSMD. */
  osmdFactory?: OsmdFactory;
}

/**
 * Renders a MusicXML score and drives a cursor to the live score position.
 *
 * Two effects, deliberately split:
 *  1. (re)load + render whenever the MusicXML changes — the expensive one.
 *  2. move the cursor whenever `cursorPosition.measure_number` changes —
 *     the cheap, frequent one. Keeping them apart means a phrase tick
 *     doesn't re-parse the whole score.
 */
export default function ScoreView({
  musicXml,
  cursorPosition,
  variant = "page",
  osmdFactory,
}: ScoreViewProps) {
  const factory =
    osmdFactory ?? (variant === "ambient" ? ambientFactory : pageFactory);
  const containerRef = useRef<HTMLDivElement>(null);
  const osmdRef = useRef<OsmdLike | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Measure the cursor currently sits on (0-based), or -1 before ready. */
  const cursorMeasureRef = useRef<number>(-1);

  // Effect 1: load + render on MusicXML change.
  useEffect(() => {
    let cancelled = false;
    setReady(false);
    setError(null);
    cursorMeasureRef.current = -1;

    if (!musicXml || !containerRef.current) {
      osmdRef.current = null;
      return;
    }

    const osmd = factory(containerRef.current);
    osmdRef.current = osmd;

    (async () => {
      try {
        await osmd.load(musicXml);
        if (cancelled) return;
        osmd.render();
        osmd.cursor.reset();
        osmd.cursor.show();
        cursorMeasureRef.current = currentMeasure(osmd);
        setReady(true);
      } catch (err) {
        if (!cancelled) {
          osmdRef.current = null;
          setError(String(err));
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [musicXml, factory]);

  // Effect 2: advance the cursor to the live measure.
  useEffect(() => {
    if (!ready) return;
    const osmd = osmdRef.current;
    if (!osmd) return;

    // ScorePosition measures are 1-based (MusicXML convention); OSMD's
    // iterator is 0-based. Park at the start when we have no position.
    const targetMeasure =
      cursorPosition === null
        ? 0
        : Math.max(0, cursorPosition.measure_number - 1);

    moveCursorToMeasure(osmd, targetMeasure, cursorMeasureRef);
  }, [ready, cursorPosition]);

  return (
    <div
      data-testid="score-view"
      className={
        variant === "ambient"
          ? "h-full w-full overflow-auto p-1"
          : "h-full w-full overflow-auto rounded-lg bg-white p-4"
      }
    >
      {error && (
        <p data-testid="score-view-error" className="text-sm text-red-600">
          Could not render this score: {error}
        </p>
      )}
      {!musicXml && !error && (
        <p data-testid="score-view-empty" className="text-sm text-gray-400">
          No score loaded.
        </p>
      )}
      <div ref={containerRef} data-testid="score-view-canvas" />
    </div>
  );
}

/** Read the cursor's current 0-based measure, or -1 if unavailable. */
function currentMeasure(osmd: OsmdLike): number {
  const idx = osmd.cursor.iterator?.currentMeasureIndex;
  return typeof idx === "number" ? idx : -1;
}

/**
 * Step the OSMD cursor forward until it reaches `targetMeasure`.
 *
 * The cursor only moves forward (OSMD has no cheap "seek backward"); if the
 * follower jumps us earlier — a repeat, or a re-alignment — we reset to the
 * start and walk forward, which is correct and bounded by the score length.
 * A guard cap prevents an infinite loop if the iterator ever stops
 * advancing (e.g. already at the end).
 */
function moveCursorToMeasure(
  osmd: OsmdLike,
  targetMeasure: number,
  cursorMeasureRef: { current: number },
): void {
  let current = cursorMeasureRef.current;

  if (targetMeasure < current) {
    osmd.cursor.reset();
    osmd.cursor.show();
    current = currentMeasure(osmd);
  }

  // Walk forward to the target. Cap iterations at a generous bound so a
  // misbehaving iterator can never hang the render loop.
  let guard = 0;
  const MAX_STEPS = 10_000;
  while (current < targetMeasure && guard < MAX_STEPS) {
    osmd.cursor.next();
    const next = currentMeasure(osmd);
    if (next === current) break; // iterator parked (end of score)
    current = next;
    guard += 1;
  }

  cursorMeasureRef.current = current;
}
