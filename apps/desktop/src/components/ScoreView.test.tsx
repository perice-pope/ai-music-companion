import { describe, it, expect, vi, beforeAll } from "vitest";
import {
  render,
  waitFor,
  cleanup,
  screen,
  fireEvent,
} from "@testing-library/react";
import ScoreView, {
  boundsFromGraphicSheet,
  measureIndexByXmlNumber,
  notationContentWidth,
  notationFitWidth,
  type OsmdLike,
  type OsmdFactory,
  type OsmdStaffMeasure,
} from "./ScoreView";
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

/** A fake with measure hit regions (#341) — three side-by-side measures. */
function makeFakeOsmdWithBounds(measureCount: number) {
  const base = makeFakeOsmd(measureCount);
  base.osmd.measureBounds = () =>
    Array.from({ length: measureCount }, (_, i) => ({
      measureNumber: i + 1,
      x: i * 100,
      y: 0,
      width: 100,
      height: 40,
    }));
  return base;
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

describe("ScoreView — ambient variant (#278)", () => {
  it("drops the white page in ambient; keeps it in page mode", async () => {
    const fakeA = makeFakeOsmd(3);
    const a = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        variant="ambient"
        osmdFactory={fakeA.factory}
      />,
    );
    expect(a.getByTestId("score-view").className).not.toContain("bg-white");
    a.unmount();

    const fakeB = makeFakeOsmd(3);
    const b = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={fakeB.factory}
      />,
    );
    expect(b.getByTestId("score-view").className).toContain("bg-white");
  });
});

describe("ScoreView — wiring with a fake OSMD", () => {
  it("loads, renders, and parks the cursor hidden while no position exists", async () => {
    const fake = makeFakeOsmd(3);
    render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.calls).toContain("render"));
    expect(fake.calls).toContain("reset");
    // A visible cursor must mean "the follower put it there" (#279):
    // with no position, lesson drills and just-loaded scores show plain
    // notation, not a parked highlight.
    expect(fake.calls).toContain("hide");
    expect(fake.calls).not.toContain("show");
    cleanup();
  });

  it("advances the cursor forward to the live measure (1-based → 0-based)", async () => {
    const fake = makeFakeOsmd(5);
    const { rerender } = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.calls).toContain("render"));

    // Move to measure 3 (1-based) → cursor should sit on index 2.
    rerender(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(3)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(2));
    cleanup();
  });

  it("walks the cursor by the XML-number map on pickup-bar scores (#370)", async () => {
    // An anacrusis score numbers its measures 0,1,2,3 — XML number n lives
    // at index n. The old "n − 1" mapping sat the cursor one measure
    // behind the player for the whole piece.
    const fake = makeFakeOsmd(4);
    fake.osmd.measureIndexMap = () =>
      new Map([
        [0, 0],
        [1, 1],
        [2, 2],
        [3, 3],
      ]);
    const { rerender } = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.calls).toContain("render"));

    // The follower reports XML measure 2 → the cursor must sit on INDEX 2
    // (the measure whose XML number was reported), not index 1.
    rerender(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(2)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(2));
    cleanup();
  });

  it("does not move backward step-by-step: resets then walks when position rewinds", async () => {
    const fake = makeFakeOsmd(5);
    const { rerender } = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(4)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(3));

    fake.calls.length = 0; // focus on the rewind behavior
    rerender(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(2)}
        osmdFactory={fake.factory}
      />,
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
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(999)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(2));
    // Parked at the last measure; must not have spun the guard cap.
    expect(nextSpy.mock.calls.length).toBeLessThan(100);
    cleanup();
  });

  it("renders an empty-state and never constructs OSMD without MusicXML", async () => {
    const fake = makeFakeOsmd(3);
    const { getByTestId } = render(
      <ScoreView
        musicXml={null}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
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
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={failing}
      />,
    );
    await waitFor(() => expect(getByTestId("score-view-error")).toBeTruthy());
    cleanup();
  });
});

/**
 * VA #279 / #324: the backend provably emitted score positions, every
 * cursor.next() "worked", and the tester still saw no cursor. Real OSMD
 * (1.9.x, pinned by inspection of the bundle) implements its follow cursor
 * as an <img> appended to the container with `position: absolute` and a
 * NEGATIVE z-index (-1/-2), painting *behind* the transparent notation SVG.
 * That only renders if the container ScoreView hands OSMD is
 *   1. positioned (`relative`) — OSMD computes the img's top/left in
 *      container-local coordinates, so the container must be the img's
 *      offset parent, and
 *   2. a stacking context (`z-0`) — otherwise the negative-z img paints
 *      behind the app's opaque backgrounds (the white page wrapper, the
 *      dark app shell) and is invisible at any coordinates.
 * jsdom neither lays out nor paints, so the honest assertable surface is
 * the contract itself: the exact element handed to the OSMD factory must
 * carry both classes. Removing either re-opens #279 in the real app.
 */
describe("ScoreView — the cursor can actually paint (#279)", () => {
  it.each(["page", "ambient"] as const)(
    "hands OSMD a positioned stacking-context container (%s variant)",
    async (variant) => {
      const fake = makeFakeOsmd(3);
      let handed: HTMLElement | null = null;
      const capturing: OsmdFactory = (container) => {
        handed = container;
        return fake.osmd;
      };
      render(
        <ScoreView
          musicXml={SCALE_XML}
          cursorPosition={null}
          variant={variant}
          osmdFactory={capturing}
        />,
      );
      await waitFor(() => expect(handed).not.toBeNull());
      const classes = (handed as unknown as HTMLElement).className.split(/\s+/);
      expect(classes).toContain("relative");
      expect(classes).toContain("z-0");
      cleanup();
    },
  );

  it("shows the cursor when the follower reports, hides it again when the position clears", async () => {
    const fake = makeFakeOsmd(5);
    const { rerender } = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.calls).toContain("render"));
    expect(fake.calls).not.toContain("show");

    // First live position → the cursor appears AND sits on the right measure.
    rerender(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(3)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(2));
    expect(fake.calls).toContain("show");

    // Session over (store clears the position) → hidden and re-parked, so a
    // stale highlight can't linger over measure 3 of a finished take.
    fake.calls.length = 0;
    rerender(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.calls).toContain("hide"));
    expect(fake.currentMeasure()).toBe(0);
    expect(fake.calls).not.toContain("show");
    cleanup();
  });

  it("calls show() once per appearance, not on every position tick", async () => {
    // Real OSMD's show() re-runs update(): getBoundingClientRect plus a
    // smooth scrollIntoView. Live following emits ~10 positions/second, so
    // a show() per tick would snap the pane back to the cursor 10×/second
    // and fight the user's own scrolling.
    const fake = makeFakeOsmd(10);
    const { rerender } = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(2)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(1));
    for (const m of [3, 4, 4, 5]) {
      rerender(
        <ScoreView
          musicXml={SCALE_XML}
          cursorPosition={pos(m)}
          osmdFactory={fake.factory}
        />,
      );
    }
    await waitFor(() => expect(fake.currentMeasure()).toBe(4));
    expect(fake.calls.filter((c) => c === "show")).toHaveLength(1);
    cleanup();
  });

  it("re-walks to a re-reported measure after the position clears (new take, same spot)", async () => {
    const fake = makeFakeOsmd(8);
    const { rerender } = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(5)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(4));
    rerender(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(0));
    // The next take reports the SAME measure. The stale-ref trap: if the
    // null branch reset the iterator but not the measure ref, the target
    // would equal the stale ref, skip the walk, and visibly park a shown
    // cursor on measure 1 while the player is at measure 5.
    rerender(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(5)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(4));
    cleanup();
  });

  it("shows the cursor again after the score reloads mid-session", async () => {
    const fake = makeFakeOsmd(5);
    const { rerender } = render(
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={pos(2)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(1));
    // Swap the score with the follower still live: the load effect re-parks
    // the cursor hidden, so the show-gate must re-arm — gating show() on a
    // null→position *transition* instead would leave the cursor invisible
    // for the rest of the session.
    rerender(
      <ScoreView
        musicXml={`${SCALE_XML}<!-- reloaded -->`}
        cursorPosition={pos(2)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() =>
      expect(fake.calls.filter((c) => c === "show")).toHaveLength(2),
    );
    expect(fake.currentMeasure()).toBe(1);
    cleanup();
  });
});

describe("ScoreView — against the real OSMD parser", () => {
  beforeAll(() => {
    // OSMD/VexFlow call getBBox during render; jsdom has no layout engine.
    const proto = (
      globalThis as unknown as {
        SVGElement?: { prototype: Record<string, unknown> };
      }
    ).SVGElement?.prototype;
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
          // OSMD's published types don't structurally match our minimal
          // `OsmdLike`, so resolve the constructor via `unknown` and assert
          // the narrow shape this test actually uses (avoids `any`).
          type OsmdCtor = new (
            container: HTMLElement,
            options: { autoResize: boolean; backend: string },
          ) => OsmdLike;
          const mod = (await import("opensheetmusicdisplay")) as unknown as {
            OpenSheetMusicDisplay?: OsmdCtor;
            default?: { OpenSheetMusicDisplay?: OsmdCtor };
          };
          const OSMD =
            mod.OpenSheetMusicDisplay ?? mod.default?.OpenSheetMusicDisplay;
          if (!OSMD) throw new Error("OpenSheetMusicDisplay export not found");
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
      <ScoreView
        musicXml={SCALE_XML}
        cursorPosition={null}
        osmdFactory={realFactory}
      />,
    );
    // Give the async load a chance; assert it did NOT land in the error state.
    await new Promise((r) => setTimeout(r, 200));
    expect(queryByTestId("score-view-error")).toBeNull();
    cleanup();
  });
});

describe("ScoreView — measure tap overlay (#341)", () => {
  // AC: tapping a measure fires the bridge with ITS number — hit regions
  // come from the layout, positioned over the notation.
  it("renders a tap target per measure and reports the tapped number", async () => {
    const fake = makeFakeOsmdWithBounds(3);
    const taps: number[] = [];
    render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={fake.factory}
        onMeasureTap={(m) => taps.push(m)}
      />,
    );
    await screen.findByTestId("measure-overlay");
    expect(screen.getAllByTestId(/measure-hit-/)).toHaveLength(3);
    fireEvent.click(screen.getByTestId("measure-hit-2"));
    expect(taps).toEqual([2]);
    const hit = screen.getByTestId("measure-hit-2");
    expect(hit).toHaveAttribute("aria-label", "Row measure 2 through 12 keys");
    expect(hit.style.left).toBe("100px");
  });

  // AC: the overlay never intercepts scroll — the wrapper ignores pointers
  // entirely; only the buttons themselves accept a tap.
  it("the overlay wrapper passes pointer events through", async () => {
    const fake = makeFakeOsmdWithBounds(2);
    render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={fake.factory}
        onMeasureTap={() => {}}
      />,
    );
    const overlay = await screen.findByTestId("measure-overlay");
    expect(overlay.className).toContain("pointer-events-none");
    for (const hit of screen.getAllByTestId(/measure-hit-/)) {
      expect(hit.className).toContain("pointer-events-auto");
    }
  });

  // AC: surfaces that don't pass the handler get NO overlay (lessons,
  // drills, read-only previews) — and a fake without bounds renders none
  // either (older shims degrade to plain notation).
  it("no handler or no bounds → no overlay", async () => {
    const withBounds = makeFakeOsmdWithBounds(2);
    const { unmount } = render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={withBounds.factory}
      />,
    );
    // Await RENDER COMPLETION first (review T3: asserting absence on the
    // first tick false-passes before the async load lands), THEN absence.
    await waitFor(() => expect(withBounds.calls).toContain("render"));
    expect(screen.queryByTestId("measure-overlay")).toBeNull();
    unmount();

    const noBounds = makeFakeOsmd(2);
    render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={noBounds.factory}
        onMeasureTap={() => {}}
      />,
    );
    await waitFor(() => expect(noBounds.calls).toContain("render"));
    expect(screen.queryByTestId("measure-overlay")).toBeNull();
  });
});

describe("boundsFromGraphicSheet — the OSMD-contract math (#341)", () => {
  const staff = (
    x: number,
    y: number,
    w: number,
    h: number,
    xmlNumber?: number,
  ): OsmdStaffMeasure => ({
    PositionAndShape: {
      AbsolutePosition: { x, y },
      Size: { width: w, height: h },
    },
    ...(xmlNumber !== undefined
      ? { parentSourceMeasure: { MeasureNumberXML: xmlNumber } }
      : {}),
  });

  // 10px per unit, scaled by zoom — the exact conversion OSMD's own
  // cursor uses. The ambient variant runs at Zoom 0.75.
  it("converts OSMD units at 10px x zoom", () => {
    const b = boundsFromGraphicSheet([[staff(2, 1, 10, 4)]], 0.75);
    expect(b).toEqual([
      { measureNumber: 1, x: 15, y: 7.5, width: 75, height: 30 },
    ]);
  });

  // Grand-staff scores: the hit region unions the measure's staves so a
  // tap anywhere in the system rows that measure.
  it("unions a measure's staves", () => {
    const b = boundsFromGraphicSheet(
      [[staff(0, 0, 10, 4), staff(0, 8, 10, 4)]],
      1,
    );
    expect(b[0]).toMatchObject({ y: 0, height: 120 });
  });

  // Review M3: pickup-bar scores number 0, 1, 2… in the XML, and the
  // backend matches THAT — the region must carry MeasureNumberXML, not
  // list order, or every tap rows the following measure.
  it("carries the XML measure number for pickup-bar scores", () => {
    const b = boundsFromGraphicSheet(
      [
        [staff(0, 0, 5, 4, 0)], // anacrusis: number="0"
        [staff(5, 0, 10, 4, 1)],
      ],
      1,
    );
    expect(b.map((r) => r.measureNumber)).toEqual([0, 1]);
  });

  // Shims without source measures fall back to list order.
  it("falls back to list order without XML numbers", () => {
    const b = boundsFromGraphicSheet([[staff(0, 0, 5, 4)]], 1);
    expect(b[0].measureNumber).toBe(1);
  });

  it("empty and shape-less inputs yield nothing", () => {
    expect(boundsFromGraphicSheet(undefined, 1)).toEqual([]);
    expect(boundsFromGraphicSheet([[{}]], 1)).toEqual([]);
  });
});

describe("measureIndexByXmlNumber — the cursor's numbering map (#370)", () => {
  const withXml = (n: number): OsmdStaffMeasure => ({
    parentSourceMeasure: { MeasureNumberXML: n },
  });

  // The follower reports the XML `number` attribute; pickup scores number
  // 0,1,2… — so XML number n lives at index n, NOT n − 1.
  it("maps pickup-bar numbering to list indices", () => {
    const map = measureIndexByXmlNumber([
      [withXml(0)], // anacrusis: number="0"
      [withXml(1)],
      [withXml(2)],
    ]);
    expect(map.get(0)).toBe(0);
    expect(map.get(1)).toBe(1);
    expect(map.get(2)).toBe(2);
  });

  it("regular scores keep the n − 1 shape", () => {
    const map = measureIndexByXmlNumber([[withXml(1)], [withXml(2)]]);
    expect(map.get(1)).toBe(0);
    expect(map.get(2)).toBe(1);
  });

  it("falls back to list order without XML numbers", () => {
    const map = measureIndexByXmlNumber([[{}], [{}]]);
    expect(map.get(1)).toBe(0);
    expect(map.get(2)).toBe(1);
  });

  // Implicit measures (after repeats / voltas) can repeat an XML number —
  // first sighting wins, so the cursor walks to the earlier statement
  // instead of teleporting past unplayed music.
  it("first sighting wins when an XML number repeats", () => {
    const map = measureIndexByXmlNumber([
      [withXml(1)],
      [withXml(1)],
      [withXml(2)],
    ]);
    expect(map.get(1)).toBe(0);
    expect(map.get(2)).toBe(2);
  });

  it("empty input yields an empty map", () => {
    expect(measureIndexByXmlNumber(undefined).size).toBe(0);
  });
});

describe("ScoreView — centered notation (VA 2026-07-14)", () => {
  // AC: notation narrower than the pane → the wrapper takes the content's
  // width (+slack) so mx-auto centers it, and the overlay rides inside the
  // same wrapper so tap regions stay aligned with the drawn measures.
  it("sizes the wrapper to the drawn content when the pane is wider", async () => {
    const fake = makeFakeOsmdWithBounds(3); // content right edge = 300px
    // Gate load so the pane width mock is in place before effect 1 reads it.
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const originalLoad = fake.osmd.load.bind(fake.osmd);
    fake.osmd.load = async (xml: string) => {
      await gate;
      return originalLoad(xml);
    };
    render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={fake.factory}
        onMeasureTap={() => {}}
      />,
    );
    Object.defineProperty(screen.getByTestId("score-view"), "clientWidth", {
      value: 1000,
      configurable: true,
    });
    release();
    const wrapper = await screen.findByTestId("notation-wrapper");
    // 300px content + 8px barline slack, centered by mx-auto.
    await waitFor(() => expect(wrapper.style.width).toBe("308px"));
    expect(wrapper.className).toContain("mx-auto");
    // Overlay coordinates are wrapper-local — hit 2 still starts at 100px.
    expect(screen.getByTestId("measure-hit-2").style.left).toBe("100px");
  });

  // AC: content as wide as the pane (or unknown — jsdom's clientWidth is 0)
  // keeps the wrapper at full width; nothing shifts, nothing clips.
  it("keeps full width when content fills the pane or layout is unknown", async () => {
    const fake = makeFakeOsmdWithBounds(3);
    render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.calls).toContain("render"));
    const wrapper = screen.getByTestId("notation-wrapper");
    expect(wrapper.style.width).toBe("");
  });
});

describe("notationFitWidth / notationContentWidth — the centering math", () => {
  const bound = (x: number, width: number) => ({
    measureNumber: 1,
    x,
    y: 0,
    width,
    height: 40,
  });

  it("content width is the farthest right edge across systems", () => {
    // Two wrapped systems: the SECOND is shorter — the union must not
    // shrink to the last rect read.
    expect(
      notationContentWidth([bound(0, 400), bound(400, 200), bound(0, 250)]),
    ).toBe(600);
    expect(notationContentWidth([])).toBe(0);
  });

  it("fits (content + slack) when the pane is meaningfully wider", () => {
    expect(notationFitWidth(1000, 300)).toBe(308);
    expect(notationFitWidth(1000, 299.4)).toBe(308); // ceil, never clip
  });

  it("declines when centering would gain nothing or inputs are unknown", () => {
    expect(notationFitWidth(320, 300)).toBeNull(); // < slack + min gain
    expect(notationFitWidth(0, 300)).toBeNull(); // jsdom / unmeasured pane
    expect(notationFitWidth(1000, 0)).toBeNull(); // no layout read
    expect(notationFitWidth(Number.NaN, 300)).toBeNull();
    expect(notationFitWidth(1000, Number.POSITIVE_INFINITY)).toBeNull();
  });
});

describe("ScoreView — centering follows window resize", () => {
  // AC: the debounced resize handler recomputes the fit width — and it now
  // runs WITHOUT a tap handler (the effect gate moved from
  // [ready, onMeasureTap] to [ready]). A pane that grows around a short
  // drill must start centering it after the 250ms debounce.
  it("recomputes the wrapper width on resize, without onMeasureTap", async () => {
    const fake = makeFakeOsmdWithBounds(3); // content right edge = 300px
    render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    // jsdom pane width is 0 at load → no fit width.
    await waitFor(() => expect(fake.calls).toContain("render"));
    const wrapper = screen.getByTestId("notation-wrapper");
    expect(wrapper.style.width).toBe("");

    // The pane grows (e.g. the window is maximized) and resize fires; the
    // 250ms-debounced handler must re-read the pane and snap the wrapper
    // to the content width. Real timers — waitFor outlives the debounce.
    Object.defineProperty(screen.getByTestId("score-view"), "clientWidth", {
      value: 1200,
      configurable: true,
    });
    fireEvent(window, new Event("resize"));
    await waitFor(() => expect(wrapper.style.width).toBe("308px"), {
      timeout: 2000,
    });
  });

  // AC2 alignment: the tap overlay must live INSIDE the sized wrapper —
  // that containment is what keeps its container-local rects aligned with
  // the centered notation (left=100px is meaningless from outside it).
  it("the measure overlay is a descendant of the sized wrapper", async () => {
    const fake = makeFakeOsmdWithBounds(2);
    render(
      <ScoreView
        musicXml="<score/>"
        cursorPosition={null}
        osmdFactory={fake.factory}
        onMeasureTap={() => {}}
      />,
    );
    const overlay = await screen.findByTestId("measure-overlay");
    expect(screen.getByTestId("notation-wrapper")).toContainElement(overlay);
  });
});
