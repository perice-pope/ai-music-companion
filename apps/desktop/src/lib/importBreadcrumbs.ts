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
 * Pure line-building lives here (unit-tested); the IPC send rides
 * `sendBreadcrumb` from positionBreadcrumbs.ts.
 */

export type ImportEntryPath = "drag-drop" | "file-picker";

export type ImportOutcome = "success" | "error" | "superseded";

/** The moment the event-independent indicator is seeded (#342). */
export function importSeededBreadcrumb(
  stage: string,
  entry: ImportEntryPath,
  ext: string,
): string {
  return `import feedback: seeded stage=${stage} via ${entry} (.${ext})`;
}

/**
 * What the indicator's DOM actually looks like at the first frame the
 * webview could paint after the seed. "MISSING" vs "present but zero
 * visible pixels" vs "fine" is the fork six VA runs couldn't answer —
 * same shape as the #279 cursor's `cursorShownBreadcrumb`.
 */
export function importFirstFrameBreadcrumb(doc: Document): string {
  const bar = doc.querySelector<HTMLElement>('[role="progressbar"]');
  if (!bar) {
    return "import feedback: first frame — indicator MISSING from DOM";
  }
  const label = bar.querySelector("span")?.textContent ?? "?";
  const r = bar.getBoundingClientRect();
  const style = doc.defaultView?.getComputedStyle(bar);
  return (
    `import feedback: first frame — "${label}" ` +
    `x=${Math.round(r.x)} y=${Math.round(r.y)} ` +
    `w=${Math.round(r.width)} h=${Math.round(r.height)} ` +
    `display=${style?.display ?? "?"} visibility=${style?.visibility ?? "?"}`
  );
}

/** The perceivable-minimum hold (#336 round three) ran its course. */
export function importHoldBreadcrumb(elapsedMs: number): string {
  return `import feedback: hold done after ${Math.round(elapsedMs)}ms`;
}

/**
 * The indicator came down (or, for a superseded import, its writes were
 * silenced while a newer import owns the screen).
 */
export function importClearedBreadcrumb(outcome: ImportOutcome): string {
  return `import feedback: cleared (${outcome})`;
}
