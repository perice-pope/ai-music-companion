/**
 * Webview-side score-position breadcrumbs (#354).
 *
 * The backend logs its half of the position stream per session (first /
 * measure change / heartbeat — `score_position_log.rs`). These helpers
 * produce the matching webview lines, sent back over the local
 * `frontend_breadcrumb` command into the SAME log file the tester pulls,
 * so one capture shows both sides of the IPC boundary line-for-line:
 * backend emitted N, webview received N (or didn't — the answer #354 has
 * been missing for five runs).
 *
 * Pure decision logic lives here (unit-tested); the IPC send is a
 * fire-and-forget that must never throw in browser/e2e contexts where
 * Tauri internals are absent.
 */
import { invoke } from "@tauri-apps/api/core";

/** Mirrors the backend's heartbeat cadence (~10 s at the 10 Hz emit). */
export const RECEIPT_HEARTBEAT_EVERY = 100;

export interface ReceiptState {
  received: number;
  lastMeasure: number | null;
}

export const freshReceiptState = (): ReceiptState => ({
  received: 0,
  lastMeasure: null,
});

/**
 * Decide the log line (if any) for one received position. First receipt
 * and measure changes always log; steady-state receipts log only at the
 * heartbeat cadence — same shape as the backend's emission log, so the
 * two streams line up.
 */
export function receiptBreadcrumb(
  state: ReceiptState,
  measure: number,
): string | null {
  state.received += 1;
  let line: string | null = null;
  if (state.received === 1) {
    line = `score-position received: first, measure=${measure}`;
  } else if (state.lastMeasure !== null && state.lastMeasure !== measure) {
    line = `score-position received: measure ${state.lastMeasure}→${measure} (received=${state.received})`;
  } else if (state.received % RECEIPT_HEARTBEAT_EVERY === 0) {
    line = `score-position still arriving (received=${state.received}, measure=${measure})`;
  }
  state.lastMeasure = measure;
  return line;
}

/**
 * What the cursor's DOM actually looks like the moment ScoreView shows
 * it. The #279 failure shape (element present, zero visible pixels) and
 * "img never created" both become one legible log line instead of a
 * tester shrug.
 */
export function cursorShownBreadcrumb(doc: Document): string {
  const img = doc.querySelector<HTMLElement>('img[id^="cursorImg"]');
  if (!img) {
    return "cursor shown: cursor img MISSING from DOM";
  }
  const r = img.getBoundingClientRect();
  const style = doc.defaultView?.getComputedStyle(img);
  return (
    `cursor shown: img x=${Math.round(r.x)} y=${Math.round(r.y)} ` +
    `w=${Math.round(r.width)} h=${Math.round(r.height)} ` +
    `display=${style?.display ?? "?"} visibility=${style?.visibility ?? "?"} z=${style?.zIndex ?? "?"}`
  );
}

/**
 * Fire-and-forget the breadcrumb to the backend log. Outside Tauri
 * (vitest jsdom, plain browser dev) the invoke either throws or rejects —
 * both are swallowed: diagnostics must never break the app they observe.
 */
export function sendBreadcrumb(message: string): void {
  try {
    void invoke("frontend_breadcrumb", { message }).catch(() => {});
  } catch {
    // No Tauri internals in this context — nothing to log to.
  }
}
