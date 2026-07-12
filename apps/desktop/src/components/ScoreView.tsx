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
  /**
   * #341: per-measure hit regions in container pixels, read from OSMD's
   * layout after render. Optional — surfaces without the overlay (and
   * older fakes) simply have no tap targets.
   */
  measureBounds?(): MeasureBound[];
}

/** One measure's hit region, container-pixel coordinates (#341). */
export interface MeasureBound {
  /** 1-based measure number, as the score model and backend count. */
  measureNumber: number;
  x: number;
  y: number;
  width: number;
  height: number;
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
const makeDefaultFactory =
  (ambient: boolean): OsmdFactory =>
  (container) => {
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
            (mod as { OpenSheetMusicDisplay?: unknown })
              .OpenSheetMusicDisplay ??
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
      measureBounds() {
        const g = inner as unknown as {
          GraphicSheet?: { MeasureList?: OsmdStaffMeasure[][] };
          Zoom?: number;
        } | null;
        return boundsFromGraphicSheet(g?.GraphicSheet?.MeasureList, g?.Zoom);
      },
    };
  };

/** The OSMD-shaped inputs the bounds reader consumes (1.9.x layout). */
export interface OsmdStaffMeasure {
  PositionAndShape?: {
    AbsolutePosition: { x: number; y: number };
    Size: { width: number; height: number };
  };
  /** OSMD's link back to the source measure — carries the XML number. */
  parentSourceMeasure?: { MeasureNumberXML?: number };
}

/**
 * #341: measure hit regions from OSMD's layout. Pure and exported so the
 * unit math — 10px × Zoom per OSMD unit (the same conversion OSMD's own
 * cursor uses), per-measure stave union, and MEASURE NUMBERING — is
 * testable without a real render (jsdom can't run OSMD's renderer).
 *
 * Numbering: the backend matches the MusicXML `number` attribute, and
 * pickup-bar scores start at 0 — so the region carries
 * `parentSourceMeasure.MeasureNumberXML` when OSMD provides it, falling
 * back to list order (i+1) only for shims that don't (review M3: index+1
 * alone rowed the WRONG measure across every pickup-bar etude).
 */
export function boundsFromGraphicSheet(
  list: OsmdStaffMeasure[][] | undefined,
  zoom: number | undefined,
): MeasureBound[] {
  if (!list) {
    return [];
  }
  const unit = 10 * (zoom ?? 1);
  const out: MeasureBound[] = [];
  list.forEach((staves, i) => {
    let x = Infinity;
    let y = Infinity;
    let right = -Infinity;
    let bottom = -Infinity;
    let xmlNumber: number | undefined;
    for (const staff of staves) {
      xmlNumber ??= staff?.parentSourceMeasure?.MeasureNumberXML;
      const p = staff?.PositionAndShape;
      if (!p) continue;
      x = Math.min(x, p.AbsolutePosition.x);
      y = Math.min(y, p.AbsolutePosition.y);
      right = Math.max(right, p.AbsolutePosition.x + p.Size.width);
      bottom = Math.max(bottom, p.AbsolutePosition.y + p.Size.height);
    }
    if (x === Infinity) return;
    out.push({
      measureNumber: xmlNumber ?? i + 1,
      x: x * unit,
      y: y * unit,
      width: (right - x) * unit,
      height: (bottom - y) * unit,
    });
  });
  return out;
}

/** Stable factory instances so effect deps don't churn between renders. */
const pageFactory = makeDefaultFactory(false);
const ambientFactory = makeDefaultFactory(true);

export interface ScoreViewProps {
  /** Raw MusicXML to render. Nothing renders while this is null. */
  musicXml: string | null;
  /**
   * Where the player currently is, from `phrase-detected`'s
   * `score_position`. The cursor advances to this measure. `null` parks
   * the cursor at the start, hidden — a visible cursor always means "the
   * follower put it there" (#279), so surfaces without live following
   * (lesson drills, a score loaded before the first position arrives)
   * show plain notation.
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
  /**
   * #341: when set, every measure grows an invisible tap target — tapping
   * rows THAT measure through 12 keys mid-practice (the in-practice half
   * of the RV bridge). Only score-follow sessions pass this; lessons and
   * drills never do.
   */
  onMeasureTap?: (measureNumber: number) => void;
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
  onMeasureTap,
}: ScoreViewProps) {
  const factory =
    osmdFactory ?? (variant === "ambient" ? ambientFactory : pageFactory);
  const containerRef = useRef<HTMLDivElement>(null);
  const osmdRef = useRef<OsmdLike | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** #341: measure hit regions, container pixels; empty until rendered. */
  const [measureRects, setMeasureRects] = useState<MeasureBound[]>([]);
  /** Measure the cursor currently sits on (0-based), or -1 before ready. */
  const cursorMeasureRef = useRef<number>(-1);
  /**
   * Whether we have show()n the cursor for the current score. Real OSMD's
   * show() re-runs update() — getBoundingClientRect + a smooth
   * scrollIntoView — so calling it on every ~10 Hz position tick would
   * snap the pane back to the cursor 10×/second and fight the user's own
   * scrolling. Only the hidden→shown transition may call show().
   */
  const cursorShownRef = useRef(false);

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
        osmd.cursor.hide();
        cursorShownRef.current = false;
        cursorMeasureRef.current = currentMeasure(osmd);
        // #341: read the layout's measure regions once per render — the
        // overlay is geometry over a static layout, not a live listener.
        setMeasureRects(osmd.measureBounds?.() ?? []);
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

  // #341 review M4: OSMD re-lays-out on window resize (autoResize), so the
  // hit regions must follow — otherwise a maximized window rows a
  // different measure than the one under the pointer. Debounced past
  // OSMD's own debounced re-render.
  useEffect(() => {
    if (!ready || !onMeasureTap) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const onResize = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        setMeasureRects(osmdRef.current?.measureBounds?.() ?? []);
      }, 250);
    };
    window.addEventListener("resize", onResize);
    return () => {
      if (timer) clearTimeout(timer);
      window.removeEventListener("resize", onResize);
    };
  }, [ready, onMeasureTap]);

  // Effect 2: advance the cursor to the live measure.
  useEffect(() => {
    if (!ready) return;
    const osmd = osmdRef.current;
    if (!osmd) return;

    // No live position (no follower installed, or the session ended):
    // park at the start and keep the cursor out of sight.
    if (cursorPosition === null) {
      if (cursorMeasureRef.current > 0) {
        osmd.cursor.reset();
        cursorMeasureRef.current = currentMeasure(osmd);
      }
      osmd.cursor.hide();
      cursorShownRef.current = false;
      return;
    }

    if (!cursorShownRef.current) {
      osmd.cursor.show();
      cursorShownRef.current = true;
    }
    // ScorePosition measures are 1-based (MusicXML convention); OSMD's
    // iterator is 0-based.
    moveCursorToMeasure(
      osmd,
      Math.max(0, cursorPosition.measure_number - 1),
      cursorMeasureRef,
    );
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
      {/* OSMD (1.9.x) appends its follow cursor to this element as an
          absolutely-positioned <img> with a NEGATIVE z-index, relying on
          the transparent notation SVG above it. `relative` makes this div
          the img's offset parent (OSMD computes the img's top/left in
          container-local coordinates); `z-0` makes it a stacking context,
          so the negative-z cursor paints above the app's backgrounds
          instead of behind them. Without both, every cursor move "works"
          but nothing is ever visible on screen (#279). */}
      <div className="relative">
        <div
          ref={containerRef}
          data-testid="score-view-canvas"
          className="relative z-0"
        />
        {/* #341: the tap overlay — transparent hit regions positioned over
            the rendered measures. The WRAPPER ignores pointers entirely
            (scroll and drag pass through untouched); only the per-measure
            buttons accept a tap. OSMD itself stays read-only. */}
        {onMeasureTap && measureRects.length > 0 && (
          <div
            data-testid="measure-overlay"
            className="pointer-events-none absolute inset-0"
          >
            {measureRects.map((r) => (
              <button
                key={r.measureNumber}
                type="button"
                data-testid={`measure-hit-${r.measureNumber}`}
                aria-label={`Row measure ${r.measureNumber} through 12 keys`}
                title={`Row measure ${r.measureNumber} through 12 keys`}
                onClick={() => onMeasureTap(r.measureNumber)}
                className="pointer-events-auto absolute rounded bg-transparent hover:bg-blue-400/10 focus-visible:bg-blue-400/15"
                style={{
                  left: r.x,
                  top: r.y,
                  width: r.width,
                  height: r.height,
                }}
              />
            ))}
          </div>
        )}
      </div>
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
