import { describe, it, expect, vi, afterEach } from "vitest";
import {
  freshReceiptState,
  receiptBreadcrumb,
  cursorShownBreadcrumb,
  sendBreadcrumb,
  RECEIPT_HEARTBEAT_EVERY,
} from "./positionBreadcrumbs";

/**
 * #354 — the webview half of the position-stream diagnostics. These lines
 * are what the VA's log pull will be read by; each test names the
 * misdiagnosis a wrong line would cause.
 */
describe("receiptBreadcrumb", () => {
  it("logs the first receipt, then stays silent inside a measure", () => {
    const state = freshReceiptState();
    expect(receiptBreadcrumb(state, 1)).toBe(
      "score-position received: first, measure=1",
    );
    // Silence between events would otherwise read as 'stream died'.
    expect(receiptBreadcrumb(state, 1)).toBeNull();
    expect(receiptBreadcrumb(state, 1)).toBeNull();
  });

  it("logs every measure change with direction and running count", () => {
    const state = freshReceiptState();
    receiptBreadcrumb(state, 1);
    receiptBreadcrumb(state, 1);
    expect(receiptBreadcrumb(state, 2)).toBe(
      "score-position received: measure 1→2 (received=3)",
    );
    // A change BACK also logs — a cursor bouncing between measures must
    // not be indistinguishable from one parked at the later measure.
    expect(receiptBreadcrumb(state, 1)).toBe(
      "score-position received: measure 2→1 (received=4)",
    );
  });

  it("heartbeats at the cadence so a steady stream proves itself", () => {
    const state = freshReceiptState();
    const lines: string[] = [];
    for (let i = 0; i < RECEIPT_HEARTBEAT_EVERY * 2 + 10; i++) {
      const line = receiptBreadcrumb(state, 3);
      if (line) lines.push(line);
    }
    expect(lines).toEqual([
      "score-position received: first, measure=3",
      `score-position still arriving (received=${RECEIPT_HEARTBEAT_EVERY}, measure=3)`,
      `score-position still arriving (received=${RECEIPT_HEARTBEAT_EVERY * 2}, measure=3)`,
    ]);
  });
});

describe("cursorShownBreadcrumb", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("reports a missing cursor img in so many words", () => {
    expect(cursorShownBreadcrumb(document)).toBe(
      "cursor shown: cursor img MISSING from DOM",
    );
  });

  it("reports the img's real geometry and computed visibility", () => {
    const img = document.createElement("img");
    img.id = "cursorImg-0";
    img.style.display = "none";
    document.body.appendChild(img);
    const line = cursorShownBreadcrumb(document);
    // jsdom geometry is all zeros — the shape of the line is the contract;
    // real-webview values are what the tester's log captures.
    expect(line).toContain("cursor shown: img x=0 y=0 w=0 h=0");
    // display:none must be legible in the log — that's the #279 shape.
    expect(line).toContain("display=none");
  });
});

describe("sendBreadcrumb", () => {
  it("never throws when Tauri internals are absent (browser/e2e)", () => {
    // jsdom has no __TAURI_INTERNALS__ — invoke throws synchronously;
    // diagnostics must not take the app down with them.
    expect(() => sendBreadcrumb("hello")).not.toThrow();
  });

  it("delivers the message over the frontend_breadcrumb command", async () => {
    const calls: Array<{ cmd: string; args: unknown }> = [];
    (
      window as unknown as { __TAURI_INTERNALS__?: unknown }
    ).__TAURI_INTERNALS__ = {
      invoke: (cmd: string, args: unknown) => {
        calls.push({ cmd, args });
        return Promise.resolve(undefined);
      },
      transformCallback: (cb: unknown) => cb,
    };
    try {
      sendBreadcrumb("cursor shown: img x=1");
      await vi.waitFor(() => expect(calls).toHaveLength(1));
      expect(calls[0].cmd).toBe("frontend_breadcrumb");
      expect(calls[0].args).toEqual({ message: "cursor shown: img x=1" });
    } finally {
      delete (window as unknown as { __TAURI_INTERNALS__?: unknown })
        .__TAURI_INTERNALS__;
    }
  });
});
