import { describe, it, expect, vi, beforeAll } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/react";
import ScoreView, { type OsmdLike, type OsmdFactory } from "./ScoreView";
import type { ScorePosition } from "../types/brain";

/**
 * A fake OSMD that records the cursor walk. `measureCount` controls how far
 * the iterator can advance — `next()` increments until the last measure,
 * then parks (matching real OSMD, which stops at the end).
 */
function makeFakeOsmd(measureCount: number) {
  const calls: string[] = [];
  let measure = 0;
  const osmd: OsmdLike = {
    async load(xml: string) {
      calls.push(`load:${xml.length}`);
    },
    render() {
      calls.push("render");
    },
    clear() {
      calls.push("clear");
    },
    cursor: {
      show() {
        calls.push("show");
      },
      hide() {
        calls.push("hide");
      },
      reset() {
        calls.push("reset");
        measure = 0;
      },
      next() {
        calls.push("next");
        if (measure < measureCount - 1) measure += 1;
      },
      get iterator() {
        return { currentMeasureIndex: measure };
      },
    },
  };
  return {
    osmd,
    calls,
    currentMeasure: () => measure,
    factory: (() => osmd) as OsmdFactory,
  };
}

const SCALE_XML = `<?xml version="1.0" encoding="UTF-8"?>
<score-partwise version="3.1">
  <part-list><score-part id="P1"><part-name>Music</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1"><attributes><divisions>1</divisions>
      <time><beats>4</beats><beat-type>4</beat-type></time>
      <clef><sign>G</sign><line>2</line></clef></attributes>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>4</duration><type>whole</type></note>
    </measure>
    <measure number="2"><note><pitch><step>D</step><octave>4</octave></pitch><duration>4</duration><type>whole</type></note></measure>
    <measure number="3"><note><pitch><step>E</step><octave>4</octave></pitch><duration>4</duration><type>whole</type></note></measure>
  </part>
</score-partwise>`;

function pos(measure: number): ScorePosition {
  return { measure_number: measure, beat: 0 };
}

describe("ScoreView — wiring with a fake OSMD", () => {
  it("loads, renders, and shows the cursor when given MusicXML", async () => {
    const fake = makeFakeOsmd(3);
    render(
      <ScoreView musicXml={SCALE_XML} cursorPosition={null} osmdFactory={fake.factory} />,
    );
    await waitFor(() => expect(fake.calls).toContain("render"));
    expect(fake.calls).toContain("reset");
    expect(fake.calls).toContain("show");
    cleanup();
  });

  it("advances the cursor forward to the live measure (1-based → 0-based)", async () => {
    const fake = makeFakeOsmd(5);
    const { rerender } = render(
      <ScoreView musicXml={SCALE_XML} cursorPosition={null} osmdFactory={fake.factory} />,
    );
    await waitFor(() => expect(fake.calls).toContain("render"));

    // Move to measure 3 (1-based) → cursor should sit on index 2.
    rerender(
      <ScoreView musicXml={SCALE_XML} cursorPosition={pos(3)} osmdFactory={fake.factory} />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(2));
    cleanup();
  });

  it("does not move backward step-by-step: resets then walks when position rewinds", async () => {
    const fake = makeFakeOsmd(5);
    const { rerender } = render(
      <ScoreView musicXml={SCALE_XML} cursorPosition={pos(4)} osmdFactory={fake.factory} />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(3));

    fake.calls.length = 0; // focus on the rewind behavior
    rerender(
      <ScoreView musicXml={SCALE_XML} cursorPosition={pos(2)} osmdFactory={fake.factory} />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(1));
    // A rewind must go through reset (forward-only cursor), not assume
    // a backward step exists.
    expect(fake.calls).toContain("reset");
    cleanup();
  });

  it("stops walking at the end of the score instead of looping forever", async () => {
    const fake = makeFakeOsmd(3); // only 3 measures (indices 0..2)
    const nextSpy = vi.spyOn(fake.osmd.cursor, "next");
    render(
      // Ask for measure 999 — way past the end.
      <ScoreView musicXml={SCALE_XML} cursorPosition={pos(999)} osmdFactory={fake.factory} />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(2));
    // Parked at the last measure; must not have spun the guard cap.
    expect(nextSpy.mock.calls.length).toBeLessThan(100);
    cleanup();
  });

  it("renders an empty-state and never constructs OSMD without MusicXML", async () => {
    const fake = makeFakeOsmd(3);
    const { getByTestId } = render(
      <ScoreView musicXml={null} cursorPosition={null} osmdFactory={fake.factory} />,
    );
    expect(getByTestId("score-view-empty")).toBeTruthy();
    expect(fake.calls).toEqual([]);
    cleanup();
  });

  it("surfaces a load error without crashing", async () => {
    const failing: OsmdFactory = () => ({
      async load() {
        throw new Error("bad xml");
      },
      render() {},
      cursor: { show() {}, hide() {}, reset() {}, next() {} },
    });
    const { getByTestId } = render(
      <ScoreView musicXml={SCALE_XML} cursorPosition={null} osmdFactory={failing} />,
    );
    await waitFor(() => expect(getByTestId("score-view-error")).toBeTruthy());
    cleanup();
  });
});

describe("ScoreView — against the real OSMD parser", () => {
  beforeAll(() => {
    // OSMD/VexFlow call getBBox during render; jsdom has no layout engine.
    const proto = (globalThis as unknown as { SVGElement?: { prototype: Record<string, unknown> } })
      .SVGElement?.prototype;
    if (proto && !proto.getBBox) {
      proto.getBBox = () => ({ x: 0, y: 0, width: 0, height: 0 });
    }
  });

  it("really parses valid MusicXML through OSMD.load (no fake)", async () => {
    // This proves the bytes we hand OSMD are actually loadable — the part
    // that matters and that a fake can't vouch for. render() can't run
    // headless, so we assert load succeeded by observing no error state.
    const realFactory: OsmdFactory = (container) => {
      let inner: OsmdLike;
      return {
        async load(xml: string) {
          const mod: any = await import("opensheetmusicdisplay");
          const OSMD = mod.OpenSheetMusicDisplay ?? mod.default?.OpenSheetMusicDisplay;
          inner = new OSMD(container, { autoResize: false, backend: "svg" });
          return inner.load(xml);
        },
        render() {
          // Swallow the headless layout failure; load() is what we assert.
          try {
            inner.render();
          } catch {
            /* jsdom has no layout engine */
          }
        },
        get cursor() {
          return (
            inner?.cursor ?? { show() {}, hide() {}, reset() {}, next() {} }
          );
        },
      };
    };

    const { queryByTestId } = render(
      <ScoreView musicXml={SCALE_XML} cursorPosition={null} osmdFactory={realFactory} />,
    );
    // Give the async load a chance; assert it did NOT land in the error state.
    await new Promise((r) => setTimeout(r, 200));
    expect(queryByTestId("score-view-error")).toBeNull();
    cleanup();
  });
});
