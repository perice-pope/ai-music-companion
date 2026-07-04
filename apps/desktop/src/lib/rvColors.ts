/**
 * The Random Variations brand palette (#278): every letter note has a color,
 * and accidentals are mixes toward the neighboring letter — ported verbatim
 * from RV (`perice-pope/random-variations`, `src/utils.tsx`) so the two apps
 * share one visual language. Musical material renders as colored cells/cards
 * keyed by pitch class.
 */

/** RV's letter-note palette (exact brand values). */
export const RV_LETTER_COLORS: Record<string, string> = {
  A: "#D783FF",
  B: "#0096FF",
  C: "#8EFA00",
  D: "#FFFB02",
  E: "#FFD479",
  F: "#FF7E79",
  G: "#FF5097",
};

/** Linear mix of two hex colors; `weight` applies to `a` (RV used mix(0.6, base, neighbor)). */
function mixHex(weight: number, a: string, b: string): string {
  const pa = parseInt(a.slice(1), 16);
  const pb = parseInt(b.slice(1), 16);
  const ch = (shift: number) =>
    Math.round(
      ((pa >> shift) & 0xff) * weight + ((pb >> shift) & 0xff) * (1 - weight),
    );
  const v = (ch(16) << 16) | (ch(8) << 8) | ch(0);
  return `#${v.toString(16).padStart(6, "0").toUpperCase()}`;
}

/**
 * Pitch class (0 = C … 11 = B) → RV color. Naturals use the letter color;
 * the five accidentals use RV's sharp mix (60% base letter, 40% next letter),
 * matching `NoteNameToColorMap["X#"]` in the original app.
 */
const PC_COLORS: string[] = (() => {
  const L = RV_LETTER_COLORS;
  const sharp = (base: string, next: string) => mixHex(0.6, L[base], L[next]);
  return [
    L.C, // 0  C
    sharp("C", "D"), // 1  C#
    L.D, // 2  D
    sharp("D", "E"), // 3  D#
    L.E, // 4  E
    L.F, // 5  F
    sharp("F", "G"), // 6  F#
    L.G, // 7  G
    sharp("G", "A"), // 8  G#
    L.A, // 9  A
    sharp("A", "B"), // 10 A#
    L.B, // 11 B
  ];
})();

/** RV color for a pitch class (0–11; larger values wrap). */
export function colorForPitchClass(pc: number): string {
  return PC_COLORS[((pc % 12) + 12) % 12];
}

/** RV color for a MIDI note. */
export function colorForMidi(midi: number): string {
  return colorForPitchClass(midi % 12);
}

const SHARP_NAMES = [
  "C",
  "C#",
  "D",
  "D#",
  "E",
  "F",
  "F#",
  "G",
  "G#",
  "A",
  "A#",
  "B",
];

/** Display name for a pitch class (sharp spelling, matching F1's labels). */
export function nameForPitchClass(pc: number): string {
  return SHARP_NAMES[((pc % 12) + 12) % 12];
}
