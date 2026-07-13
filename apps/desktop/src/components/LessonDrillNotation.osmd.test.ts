import { describe, it, expect, beforeAll } from "vitest";
import { OpenSheetMusicDisplay } from "opensheetmusicdisplay";
import fixtureRaw from "../test-fixtures/emitted-lesson-drills.json?raw";

/**
 * #327 — every lesson-drill shape through the REAL OSMD parser.
 *
 * VA runs #324/#327 saw *"undefined is not an object (evaluating
 * 't3.StaffEntries')"* on some lesson keys but not others. The shape behind
 * that crash is a measure OSMD parses with no staff entries (the #356
 * invalid-`<direction>` emission produced exactly that in measure 1 of
 * every drill). This sweep loads the drill generator's actual output —
 * all 12 tonics × all four drill kinds, plus the full difficulty ladder
 * and stacked block chords at the two VA-sighted keys — and walks the
 * exact `StaffEntries` chain from the error for every entry.
 *
 * The fixture is pinned to the Rust generator+emitter by
 * `crates/brain/tests/lesson_drill_notation_test.rs`, so this can never
 * silently test stale XML. `render()` stays out for the same reason as in
 * `EmittedNotation.osmd.test.ts`: OSMD's layout pass needs a rasterizing
 * canvas jsdom doesn't have. The parse model this reads is what the cursor
 * and layout consume — a measure that parses complete cannot present the
 * empty-StaffEntries shape.
 */

interface DrillFixtureEntry {
  id: string;
  fifths: number;
  sounding_per_measure: number[];
  music_xml: string;
}

const entries = JSON.parse(fixtureRaw) as DrillFixtureEntry[];

beforeAll(() => {
  // jsdom has no canvas. OSMD's post-parse graphic pass wants a 2D context
  // only to measure text widths; a fixed-width stub keeps that pass quiet
  // and deterministic.
  HTMLCanvasElement.prototype.getContext = (() => ({
    font: "",
    measureText: (text: string) => ({ width: 10 * text.length }),
  })) as unknown as typeof HTMLCanvasElement.prototype.getContext;
});

/**
 * Sounding (non-rest) notes OSMD parsed, per source measure — via the same
 * `VerticalSourceStaffEntryContainers[].StaffEntries` chain the VA crash
 * named. A container whose StaffEntries is missing throws right here,
 * turning the field TypeError into a named CI failure.
 */
async function osmdSoundingPerMeasure(xml: string): Promise<number[]> {
  const osmd = new OpenSheetMusicDisplay(document.createElement("div"), {
    autoResize: false,
    backend: "svg",
  });
  await osmd.load(xml);
  return osmd.Sheet.SourceMeasures.map((measure) => {
    let sounding = 0;
    for (const container of measure.VerticalSourceStaffEntryContainers) {
      for (const staffEntry of container.StaffEntries) {
        if (!staffEntry) continue;
        for (const voiceEntry of staffEntry.VoiceEntries) {
          sounding += voiceEntry.Notes.filter((note) => !note.isRest()).length;
        }
      }
    }
    return sounding;
  });
}

describe("lesson-drill MusicXML through the real OSMD parser (#327)", () => {
  it("the fixture carries the full sweep", () => {
    // 12 tonics × 4 kinds on the key axis, plus the ladder + stacked
    // shapes at the two VA-sighted keys. A shrunken fixture would pass
    // every per-entry test below while silently covering nothing.
    expect(entries.length).toBe(84);
    const tonics = new Set(entries.map((e) => e.id.split("-")[0]));
    expect(tonics.size).toBe(12);
    expect(entries.some((e) => e.id.endsWith("-stacked"))).toBe(true);
  });

  for (const entry of entries) {
    it(`${entry.id} parses complete — no measure loses its notes`, async () => {
      const perMeasure = await osmdSoundingPerMeasure(entry.music_xml);
      // Exactly what the Rust score model engraved, measure by measure —
      // a blank measure 1 (the #324/#327 crash shape) or a dropped chord
      // tone fails here with the drill's coordinates in the test name.
      expect(perMeasure).toEqual(entry.sounding_per_measure);
    });
  }
});
