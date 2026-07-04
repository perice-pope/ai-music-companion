import { describe, it, expect } from "vitest";
import {
  RV_LETTER_COLORS,
  colorForPitchClass,
  colorForMidi,
  nameForPitchClass,
} from "./rvColors";

describe("rvColors — the RV brand palette (#278)", () => {
  // The letter colors are the brand — they must match RV's utils.tsx exactly.
  it("naturals use RV's exact letter colors", () => {
    expect(colorForPitchClass(0)).toBe("#8EFA00"); // C
    expect(colorForPitchClass(2)).toBe("#FFFB02"); // D
    expect(colorForPitchClass(4)).toBe("#FFD479"); // E
    expect(colorForPitchClass(5)).toBe("#FF7E79"); // F
    expect(colorForPitchClass(7)).toBe("#FF5097"); // G
    expect(colorForPitchClass(9)).toBe("#D783FF"); // A
    expect(colorForPitchClass(11)).toBe("#0096FF"); // B
  });

  // Accidentals are RV's 60/40 mix toward the next letter — C# must sit
  // between C green and D yellow, distinct from both.
  it("accidentals are mixes between neighbor letters", () => {
    const cSharp = colorForPitchClass(1);
    expect(cSharp).not.toBe(RV_LETTER_COLORS.C);
    expect(cSharp).not.toBe(RV_LETTER_COLORS.D);
    // 0.6*0x8E + 0.4*0xFF = 0xBB red channel — pins the mix math + weight.
    expect(cSharp).toBe("#BBFA01");
  });

  it("midi maps through pitch class and wraps octaves", () => {
    expect(colorForMidi(60)).toBe(colorForPitchClass(0)); // C4
    expect(colorForMidi(72)).toBe(colorForMidi(60)); // octave-equivalent
    expect(colorForMidi(70)).toBe(colorForPitchClass(10)); // Bb4
  });

  it("names pitch classes", () => {
    expect(nameForPitchClass(0)).toBe("C");
    expect(nameForPitchClass(10)).toBe("A#");
    expect(nameForPitchClass(12)).toBe("C");
  });
});
