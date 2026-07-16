import { describe, it, expect, afterEach } from "vitest";
import {
  importClearedBreadcrumb,
  importFirstFrameBreadcrumb,
  importHoldBreadcrumb,
  importSeededBreadcrumb,
} from "./importBreadcrumbs";

// #336: the log lines are the product here — a tester pulls them verbatim
// to answer "which of the four moments never happens in the real shell",
// so each test pins the exact line a moment produces.
describe("importSeededBreadcrumb", () => {
  it("names the stage, the entry path, and the extension", () => {
    expect(importSeededBreadcrumb("transcribing", "file-picker", "wav")).toBe(
      "import feedback: seeded stage=transcribing via file-picker (.wav)",
    );
  });

  it("distinguishes the drag-drop entry path", () => {
    expect(importSeededBreadcrumb("reading-notes", "drag-drop", "pdf")).toBe(
      "import feedback: seeded stage=reading-notes via drag-drop (.pdf)",
    );
  });
});

describe("importFirstFrameBreadcrumb", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("says MISSING when no indicator is in the DOM", () => {
    expect(importFirstFrameBreadcrumb(document)).toBe(
      "import feedback: first frame — indicator MISSING from DOM",
    );
  });

  it("reports the visible label and real geometry of a present indicator", () => {
    const bar = document.createElement("div");
    bar.setAttribute("role", "progressbar");
    const label = document.createElement("span");
    label.textContent = "Listening for notes…";
    bar.appendChild(label);
    document.body.appendChild(bar);
    // jsdom rects are all-zero; substitute measured values so the test
    // proves the line carries the element's geometry, not constants.
    bar.getBoundingClientRect = () =>
      ({ x: 24.4, y: 511.6, width: 320, height: 8 }) as DOMRect;

    expect(importFirstFrameBreadcrumb(document)).toBe(
      'import feedback: first frame — "Listening for notes…" ' +
        "x=24 y=512 w=320 h=8 display=block visibility=visible",
    );
  });

  it("still logs geometry when the indicator has no label span", () => {
    const bar = document.createElement("div");
    bar.setAttribute("role", "progressbar");
    document.body.appendChild(bar);

    expect(importFirstFrameBreadcrumb(document)).toBe(
      'import feedback: first frame — "?" ' +
        "x=0 y=0 w=0 h=0 display=block visibility=visible",
    );
  });
});

describe("importHoldBreadcrumb", () => {
  it("logs the elapsed hold rounded to whole milliseconds", () => {
    expect(importHoldBreadcrumb(1204.6)).toBe(
      "import feedback: hold done after 1205ms",
    );
  });
});

describe("importClearedBreadcrumb", () => {
  it("names each lifecycle exit", () => {
    expect(importClearedBreadcrumb("success")).toBe(
      "import feedback: cleared (success)",
    );
    expect(importClearedBreadcrumb("error")).toBe(
      "import feedback: cleared (error)",
    );
    expect(importClearedBreadcrumb("superseded")).toBe(
      "import feedback: cleared (superseded)",
    );
  });
});
