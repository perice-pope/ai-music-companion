import { describe, it, expect } from "vitest";
import { keyName, spelledTonicName } from "./brain";
import type { KeyEstimate, Mode } from "./brain";

function key(tonic: number, mode: Mode): KeyEstimate {
  return { tonic, mode, confidence: 0.8, margin: 0.2 };
}

// #387: frontend-rendered key names (recap header) must use the same
// conventional spelling as the backend strip names — flat keys name flats.
// These mirror `theory::key::tests`, so a drift between the two tables
// shows up as a red test on whichever side changed.
describe("keyName", () => {
  it("names flat keys on the flat side, never sharp-side impossibles", () => {
    expect(keyName(key(8, "ionian"))).toBe("Ab major");
    expect(keyName(key(1, "ionian"))).toBe("Db major");
    expect(keyName(key(3, "ionian"))).toBe("Eb major");
    expect(keyName(key(10, "ionian"))).toBe("Bb major");
    expect(keyName(key(10, "aeolian"))).toBe("Bb minor");
    expect(keyName(key(3, "mixolydian"))).toBe("Eb Mixolydian");
    // The positive wrap: F#+Lydian is raw +7 fifths → Gb Lydian (5 flats).
    expect(keyName(key(6, "lydian"))).toBe("Gb Lydian");
  });

  it("matches the backend spelling for every remaining mode offset", () => {
    // One pair per mode not covered above, pinned to the values
    // theory::key_fifths produces — a drifted MODE_FIFTHS_OFFSET entry
    // fails here even though ionian/aeolian tests still pass.
    expect(keyName(key(1, "dorian"))).toBe("C# Dorian");
    expect(keyName(key(11, "phrygian"))).toBe("B Phrygian");
    expect(keyName(key(10, "locrian"))).toBe("A# Locrian");
  });

  it("keeps sharp-conventional keys sharp via the enharmonic wrap", () => {
    expect(keyName(key(6, "ionian"))).toBe("F# major");
    expect(keyName(key(1, "aeolian"))).toBe("C# minor");
    expect(keyName(key(8, "aeolian"))).toBe("G# minor");
  });

  it("leaves natural-named keys untouched", () => {
    expect(keyName(key(0, "ionian"))).toBe("C major");
    expect(keyName(key(9, "aeolian"))).toBe("A minor");
    expect(keyName(key(7, "mixolydian"))).toBe("G Mixolydian");
  });

  it("wraps out-of-range tonics mod 12", () => {
    expect(spelledTonicName(20, "ionian")).toBe("Ab");
    expect(spelledTonicName(-4, "ionian")).toBe("Ab");
  });
});
