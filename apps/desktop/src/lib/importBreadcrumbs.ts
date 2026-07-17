/**
 * Webview-side import-feedback breadcrumbs (#336).
 *
 * Six VA runs never saw the .wav loading message; every unit and
 * real-browser E2E test shows it painting. The divergence is
 * real-shell-specific (WKWebView), so the answer has to come from the
 * shell itself: these lines mark the four moments of an import's feedback
 * lifecycle — indicator seeded, first paintable frame after the seed
 * (rAF), hold elapsed, indicator cleared — plus which entry path the
 * tester used (drag vs picker), in the same tester-pulled log as the #354
 * position breadcrumbs. One log pull then shows which moment never
 * happens on her machine.
 *
 * Every line carries the import's sequence number (`#N`), so overlapping
 * imports (a tester who sees nothing retries fast) stay attributable.
 *
 * Pure line-building lives here (unit-tested); the IPC send rides
 * `sendBreadcrumb` from positionBreadcrumbs.ts.
 */

export type ImportEntryPath = "drag-drop" | "file-picker";

export type ImportOutcome = "success" | "error";

/** The moment the event-independent indicator is seeded (#342). */
export function importSeededBreadcrumb(
  seq: number,
  stage: string,
  entry: ImportEntryPath,
  ext: string,
): string {
  return `import feedback #${seq}: seeded stage=${stage} via ${entry} (.${ext})`;
}

/**
 * What the indicator's DOM actually shows right now, scoped to the drop
 * zone so a future progressbar elsewhere can't repoint the diagnostic.
 * "MISSING" vs "present but zero visible pixels" vs "fine" is the fork
 * six VA runs couldn't answer — same shape as the #279 cursor's
 * `cursorShownBreadcrumb`.
 */
export function describeImportIndicator(root: Element): string {
  const bar = root.querySelector<HTMLElement>('[role="progressbar"]');
  if (!bar) {
    return "indicator MISSING from DOM";
  }
  const label = bar.querySelector("span")?.textContent ?? "?";
  const r = bar.getBoundingClientRect();
  const style = root.ownerDocument.defaultView?.getComputedStyle(bar);
  return (
    `"${label}" x=${Math.round(r.x)} y=${Math.round(r.y)} ` +
    `w=${Math.round(r.width)} h=${Math.round(r.height)} ` +
    `display=${style?.display ?? "?"} visibility=${style?.visibility ?? "?"}`
  );
}

/**
 * Probe the first frame the shell could paint after the seed. React's
 * commit rides a MessageChannel task, which the first rAF can beat — a
 * lone MISSING there would be a false alarm, the one wrong answer this
 * diagnostic must not produce. So a MISSING first sample waits one more
 * frame and reports both on a single line.
 */
export function scheduleImportFirstFrameProbe(
  seq: number,
  root: Element | null,
  send: (line: string) => void,
): void {
  const win = root?.ownerDocument.defaultView;
  if (!root || !win) return;
  win.requestAnimationFrame(() => {
    const first = describeImportIndicator(root);
    if (first !== "indicator MISSING from DOM") {
      send(`import feedback #${seq}: first frame — ${first}`);
      return;
    }
    win.requestAnimationFrame(() => {
      send(
        `import feedback #${seq}: first frame — ${first}; ` +
          `second frame — ${describeImportIndicator(root)}`,
      );
    });
  });
}

/** The perceivable-minimum hold (#336 round three) ran its course. */
export function importHoldBreadcrumb(seq: number, elapsedMs: number): string {
  return `import feedback #${seq}: hold done after ${Math.round(elapsedMs)}ms`;
}

/**
 * The indicator came down. A superseded import (a newer one owns the
 * screen; its own writes were silenced) still reports how it would have
 * ended, so no exit is ambiguous in the log.
 */
export function importClearedBreadcrumb(
  seq: number,
  outcome: ImportOutcome,
  superseded: boolean,
): string {
  return `import feedback #${seq}: cleared (${
    superseded ? `superseded, was ${outcome}` : outcome
  })`;
}
