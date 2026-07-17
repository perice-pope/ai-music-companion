import { describe, it, expect, afterEach } from "vitest";
import {
  describeImportIndicator,
  importClearedBreadcrumb,
  importHoldBreadcrumb,
  importSeededBreadcrumb,
  scheduleImportFirstFrameProbe,
} from "./importBreadcrumbs";

// #336: the log lines are the product here — a tester pulls them verbatim
// to answer "which of the four moments never happens in the real shell",
// so each test pins the exact line a moment produces.

function indicator(label?: string): HTMLElement {
  const bar = document.createElement("div");
  bar.setAttribute("role", "progressbar");
  if (label !== undefined) {
    const span = document.createElement("span");
    span.textContent = label;
    bar.appendChild(span);
  }
  return bar;
}

const nextFrame = () =>
  new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

afterEach(() => {
  document.body.innerHTML = "";
});

describe("importSeededBreadcrumb", () => {
  it("names the import, the stage, the entry path, and the extension", () => {
    expect(
      importSeededBreadcrumb(3, "transcribing", "file-picker", "wav"),
    ).toBe(
      "import feedback #3: seeded stage=transcribing via file-picker (.wav)",
    );
  });

  it("distinguishes the drag-drop entry path", () => {
    expect(importSeededBreadcrumb(1, "reading-notes", "drag-drop", "pdf")).toBe(
      "import feedback #1: seeded stage=reading-notes via drag-drop (.pdf)",
    );
  });
});

describe("describeImportIndicator", () => {
  it("says MISSING when no indicator is under the root", () => {
    expect(describeImportIndicator(document.body)).toBe(
      "indicator MISSING from DOM",
    );
  });

  it("reports the visible label and real geometry of a present indicator", () => {
    const bar = indicator("Listening for notes…");
    document.body.appendChild(bar);
    // jsdom rects are all-zero; substitute measured values so the test
    // proves the line carries the element's geometry, not constants.
    bar.getBoundingClientRect = () =>
      ({ x: 24.4, y: 511.6, width: 320, height: 8 }) as DOMRect;

    expect(describeImportIndicator(document.body)).toBe(
      '"Listening for notes…" x=24 y=512 w=320 h=8 ' +
        "display=block visibility=visible",
    );
  });

  it("still reports geometry when the indicator has no label span", () => {
    document.body.appendChild(indicator());

    expect(describeImportIndicator(document.body)).toBe(
      '"?" x=0 y=0 w=0 h=0 display=block visibility=visible',
    );
  });

  it("only sees indicators under the given root", () => {
    const elsewhere = document.createElement("div");
    elsewhere.appendChild(indicator("Updating…"));
    document.body.appendChild(elsewhere);
    const root = document.createElement("div");
    document.body.appendChild(root);

    expect(describeImportIndicator(root)).toBe("indicator MISSING from DOM");
  });
});

describe("scheduleImportFirstFrameProbe", () => {
  it("sends one line when the indicator is present at the first frame", async () => {
    const root = document.createElement("div");
    root.appendChild(indicator("Listening for notes…"));
    document.body.appendChild(root);
    const sent: string[] = [];

    scheduleImportFirstFrameProbe(1, root, (line) => sent.push(line));
    await nextFrame();
    await nextFrame();

    expect(sent).toEqual([
      'import feedback #1: first frame — "Listening for notes…" ' +
        "x=0 y=0 w=0 h=0 display=block visibility=visible",
    ]);
  });

  it("samples a second frame before reporting MISSING — a rAF can beat React's commit", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const sent: string[] = [];

    scheduleImportFirstFrameProbe(2, root, (line) => sent.push(line));
    // Lands in the same frame as the probe's first (MISSING) sample —
    // exactly how a React commit slips in between the two samples.
    requestAnimationFrame(() =>
      root.appendChild(indicator("Listening for notes…")),
    );
    await nextFrame();
    await nextFrame();
    await nextFrame();

    expect(sent).toEqual([
      "import feedback #2: first frame — indicator MISSING from DOM; " +
        'second frame — "Listening for notes…" ' +
        "x=0 y=0 w=0 h=0 display=block visibility=visible",
    ]);
  });

  it("reports MISSING on both frames when the indicator never appears", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const sent: string[] = [];

    scheduleImportFirstFrameProbe(5, root, (line) => sent.push(line));
    await nextFrame();
    await nextFrame();
    await nextFrame();

    expect(sent).toEqual([
      "import feedback #5: first frame — indicator MISSING from DOM; " +
        "second frame — indicator MISSING from DOM",
    ]);
  });

  it("does nothing for a null root — diagnostics never break the app", async () => {
    const sent: string[] = [];
    scheduleImportFirstFrameProbe(1, null, (line) => sent.push(line));
    await nextFrame();
    await nextFrame();

    expect(sent).toEqual([]);
  });
});

describe("importHoldBreadcrumb", () => {
  it("logs the elapsed hold rounded to whole milliseconds", () => {
    expect(importHoldBreadcrumb(1, 1204.6)).toBe(
      "import feedback #1: hold done after 1205ms",
    );
  });
});

describe("importClearedBreadcrumb", () => {
  it("names each lifecycle exit", () => {
    expect(importClearedBreadcrumb(1, "success", false)).toBe(
      "import feedback #1: cleared (success)",
    );
    expect(importClearedBreadcrumb(2, "error", false)).toBe(
      "import feedback #2: cleared (error)",
    );
  });

  it("keeps a superseded import's own ending attributable", () => {
    expect(importClearedBreadcrumb(1, "success", true)).toBe(
      "import feedback #1: cleared (superseded, was success)",
    );
    expect(importClearedBreadcrumb(1, "error", true)).toBe(
      "import feedback #1: cleared (superseded, was error)",
    );
  });
});
