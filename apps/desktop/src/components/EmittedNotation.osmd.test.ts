import { describe, it, expect, beforeAll } from "vitest";
import { OpenSheetMusicDisplay } from "opensheetmusicdisplay";
import trumpetXml from "../test-fixtures/emitted-band-trumpet.musicxml?raw";
import bassXml from "../test-fixtures/emitted-band-bass.musicxml?raw";
import scaleXml from "../test-fixtures/emitted-scale-score.musicxml?raw";

/**
 * #356 — the emitted-notation ⇄ OSMD contract, tested against the REAL
 * OSMD parser (its `load()` works under jsdom; only `render()` needs a
 * layout engine).
 *
 * The fixtures are the Rust emitter's exact output for the va-testing-kit
 * sources, pinned by `crates/brain/tests/emitted_notation_test.rs` so they
 * cannot drift from the emitter. The VA finding this guards: an emitted
 * `<direction>` without a `<direction-type>` child made OSMD 1.9.x silently
 * drop EVERY note of measure 1 — imported scores rendered half their notes
 * — and the staff label fell back to OSMD's anonymous "Music".
 */

beforeAll(() => {
  // jsdom has no canvas. OSMD's post-parse graphic pass wants a 2D context
  // only to measure text widths; a fixed-width stub keeps that pass quiet
  // and deterministic. The assertions below read the parsed `Sheet` model,
  // which is complete before any graphics run.
  HTMLCanvasElement.prototype.getContext = (() => ({
    font: "",
    measureText: (text: string) => ({ width: 10 * text.length }),
  })) as unknown as typeof HTMLCanvasElement.prototype.getContext;
});

/** Count the sounding (non-rest) notes OSMD parsed, per source measure. */
async function osmdSoundingPerMeasure(
  xml: string,
): Promise<{ perMeasure: number[]; partName: string | undefined }> {
  const osmd = new OpenSheetMusicDisplay(document.createElement("div"), {
    autoResize: false,
    backend: "svg",
  });
  await osmd.load(xml);
  const sheet = osmd.Sheet;
  const perMeasure = sheet.SourceMeasures.map((measure) => {
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
  return { perMeasure, partName: sheet.Instruments[0]?.Name };
}

describe("emitted MusicXML through the real OSMD parser (#356)", () => {
  it("keeps all 8 band-Trumpet notes — measure 1 must not go blank", async () => {
    const { perMeasure, partName } = await osmdSoundingPerMeasure(trumpetXml);
    expect(perMeasure).toEqual([4, 4]);
    expect(partName).toBe("Trumpet");
  });

  it("keeps all 4 band-Bass notes and the Bass label", async () => {
    const { perMeasure, partName } = await osmdSoundingPerMeasure(bassXml);
    expect(perMeasure).toEqual([2, 2]);
    expect(partName).toBe("Bass");
  });

  it("keeps all 16 notes of the round-tripped scale score", async () => {
    const { perMeasure, partName } = await osmdSoundingPerMeasure(scaleXml);
    expect(perMeasure).toEqual([4, 4, 4, 4]);
    expect(partName).toBe("Melody");
  });

  it("regression: a direction without direction-type blanks the measure", async () => {
    // The exact malformed shape the emitter used to produce. If an OSMD
    // upgrade makes this pass with [4, 4], the library stopped choking on
    // the shape — the emitter fix stays correct regardless (the shape is
    // invalid MusicXML), so simply DELETE this one test and note the OSMD
    // version in the commit message.
    const malformed = trumpetXml.replace(
      /<direction placement="above">[\s\S]*?<\/direction>/,
      '<direction placement="above">\n        <sound tempo="100"/>\n      </direction>',
    );
    expect(malformed).not.toBe(trumpetXml);
    const { perMeasure } = await osmdSoundingPerMeasure(malformed);
    expect(perMeasure).toEqual([0, 4]);
  });
});
