import { useEffect, useRef, useState } from "react";
import type { NoteEdit } from "../types/brain";

/** localStorage key for the device-local rhythm-layer preference. */
const RHYTHM_PREF_KEY = "amc.cellstaff.showRhythms";

function readRhythmPref(): boolean {
  try {
    return window.localStorage.getItem(RHYTHM_PREF_KEY) === "1";
  } catch {
    return false;
  }
}
import type { CellStaffViewDto, CellStaffNoteDto } from "../types/brain";
import { colorForPitchClass, RV_NON_ROOT_FILL } from "../lib/rvColors";

/**
 * CellStaff (#292 slice 1): the RV dot staff. Stemless noteheads colored by
 * pitch class on five lines — pure geometry over a backend-computed view
 * (steps, spelling, accidentals all arrive from Rust; this file contains no
 * music theory). Renders SYSTEMS of 4 measures per line, stacked — like a
 * printed page, never a pager (#445-5: "people don't scroll through music").
 */

/** Vertical gap between staff lines (px in the SVG's own units). */
const LINE_GAP = 10;
/** One diatonic step = half a line gap. */
const STEP = LINE_GAP / 2;
/** y of the bottom staff line (step 0); higher steps go UP (smaller y). */
const BOTTOM_Y = 5 * LINE_GAP;
/** Measures per system (one printed line). */
export const MEASURES_PER_SYSTEM = 4;

const WIDTH = 640;
/** Where key-signature glyphs start (right of the clef). */
const CLEF_ZONE = 38;
/** Horizontal advance per key-signature glyph. */
const KEYSIG_STEP = 8;
/** Breathing room between the last signature glyph and the first barline —
 * the barline never starts until the signature has fully ended (#445-1). */
const KEYSIG_GAP = 12;
/** Note-column insets from a measure's barlines. The LEFT inset also covers
 * the accidental engraved against the column (text ends at x-11, glyph body
 * ~8px wide), so an accidental can't touch the barline either. */
const NOTE_INSET_L = 22;
/** The RIGHT inset also covers a stacked second's head shift (+9) and its
 * ledger-line overhang (+9 from the shifted head) — review N1: the intent
 * is notes live INSIDE their measure, shifted heads included. */
const NOTE_INSET_R = 24;
/** #471-1 accidental collision model. An in-line glyph anchors
 * textAnchor=end at column x−ACC_ANCHOR with ~ACC_W px of body: box
 * [x−19, x−11]. A notehead is cx±HEAD_RX / cy±HEAD_RY. Measured: adjacent
 * columns need ≥ 26.5px for ACC_CLEAR of daylight (24.5 = touching), but a
 * full eighth-note measure spaces columns ~12.3px — so dense chromatic
 * bars CANNOT be solved horizontally inside the founder's fixed-width
 * no-scroll measures (8 columns would need 185.5px of the 98.5px span).
 * Colliding glyphs instead HOIST: cue size, centered above their own
 * column (the editorial/ficta convention). The noteheads NEVER move — in
 * the default stemless view horizontal position is the only rhythm signal,
 * and widening accidental columns would fake a syncopation. */
const ACC_ANCHOR = 11;
const ACC_W = 8;
/** Half-height of the in-line glyph's ink (fontSize 13). */
const ACC_HALF_H = 6.5;
const HEAD_RX = 5.5;
const HEAD_RY = 4.2;
/** A glyph within this of a foreign notehead on BOTH axes is a collision —
 * vertical separation alone (≥ 3 staff steps at quarter spacing) keeps a
 * glyph honestly in-line. */
const ACC_CLEAR = 2;
const ACC_CUE_SIZE = 10;
/** Cue-glyph ink half-width (fontSize 10, textAnchor=middle). */
const ACC_CUE_HALF_W = 3;
/** Cue-glyph ink half-height (fontSize 10). */
const ACC_CUE_HALF_H = 5;
/** Head-center → hoisted-glyph-center rise: HEAD_RY + ACC_CLEAR +
 * ACC_CUE_HALF_H, rounded up so the glyph clears its own head. */
const ACC_DODGE_RISE = 12;
/** Extra rise per additional hoisted glyph stacked over one column. */
const ACC_DODGE_STACK = 11;
/** Vertical gap between stacked systems. */
const SYSTEM_GAP = LINE_GAP;
const RIGHT_PAD = 12;
const HEIGHT = BOTTOM_Y + 4 * LINE_GAP; // headroom for ledger lines both ways
/** Default vertical extent; grows when notes ride ledger lines beyond it so
 * an 8va-edited note can never vanish off-canvas (review must-fix). The
 * computed value is shared with stepPx's drag quantization (review M6). */
const DEFAULT_MIN_Y = -2 * LINE_GAP;
const DEFAULT_MAX_Y = HEIGHT + LINE_GAP + DEFAULT_MIN_Y;

/** ViewBox [minY, height] covering every visible step, with padding. */
function viewBoxFor(steps: number[]): [number, number] {
  const pad = LINE_GAP;
  let minY = DEFAULT_MIN_Y;
  let maxY = DEFAULT_MAX_Y;
  for (const s of steps) {
    minY = Math.min(minY, yFor(s) - pad);
    maxY = Math.max(maxY, yFor(s) + pad);
  }
  return [minY, maxY - minY];
}

/** Staff steps carrying each sharp/flat of a treble key signature. */
const SHARP_STEPS = [8, 5, 9, 6, 3, 7, 4];
const FLAT_STEPS = [4, 7, 3, 6, 2, 5, 1];

const yFor = (step: number) => BOTTOM_Y - step * STEP;

/** Ledger line steps a note at `step` needs (even steps outside the staff). */
function ledgerSteps(step: number): number[] {
  const lines: number[] = [];
  for (let s = -2; s >= step; s -= 2) lines.push(s);
  for (let s = 10; s <= step; s += 2) lines.push(s);
  return lines;
}

function accidentalGlyph(alter: number): string {
  if (alter > 1) return "𝄪";
  if (alter === 1) return "♯";
  if (alter === -1) return "♭";
  if (alter < -1) return "𝄫";
  return "♮";
}

function Dot({
  note,
  x,
  headOffset = 0,
  accDodgeY,
  showRhythms,
  selected,
  ghostSteps,
  onPointerDown,
}: {
  note: CellStaffNoteDto;
  /** The beat column's x. Accidentals engrave against THIS (column-left). */
  x: number;
  /** #349 T2a: rightward notehead shift for the upper note of a stacked
   * second. Moves the head/halo/stem — never the accidental, which stays
   * left of the whole chord column per engraving convention. */
  headOffset?: number;
  /** #471-1: when set, the accidental HOISTS — cue glyph centered above the
   * column at this y — because its in-line box would sit on a neighboring
   * notehead. Undefined = the classic in-line engraving, byte-identical. */
  accDodgeY?: number;
  showRhythms: boolean;
  selected: boolean;
  /** Live drag preview: diatonic steps the ghost has moved (0 = at rest). */
  ghostSteps: number;
  onPointerDown?: (e: React.PointerEvent) => void;
}) {
  const y = yFor(note.step + ghostSteps);
  const headX = x + headOffset;
  // Rhythm layer (#292 slice 2): stems/flags are drawn ON the same dot at the
  // same position — the layer NEVER moves a notehead. Whole notes stay bare.
  const stemUp = note.step < 4;
  const stemX = stemUp ? headX + 5 : headX - 5;
  const stemY2 = stemUp ? y - 3.2 * LINE_GAP : y + 3.2 * LINE_GAP;
  const wantsStem = showRhythms && note.duration_beats < 4;
  const wantsFlag = showRhythms && note.duration_beats < 1;
  return (
    <g
      data-testid={`staff-note-${note.midi}-${note.start_beat}`}
      onPointerDown={onPointerDown}
      className={onPointerDown ? "cursor-grab" : undefined}
    >
      {ledgerSteps(note.step).map((s) => (
        <line
          key={s}
          x1={headX - 9}
          x2={headX + 9}
          y1={yFor(s)}
          y2={yFor(s)}
          stroke="#9CA3AF"
          strokeWidth={1}
        />
      ))}
      {note.accidental !== null &&
        (accDodgeY === undefined ? (
          <text
            x={x - ACC_ANCHOR}
            y={y}
            textAnchor="end"
            dominantBaseline="central"
            className="fill-gray-300"
            fontSize={13}
            data-testid="staff-accidental"
          >
            {accidentalGlyph(note.accidental)}
          </text>
        ) : (
          // #471-1 hoisted form: the in-line slot is occupied by a neighbor
          // notehead, so the glyph rides above its own column — cue size,
          // the editorial (ficta) placement. Follows the drag ghost like
          // the in-line form does.
          <text
            x={x}
            y={accDodgeY - ghostSteps * STEP}
            textAnchor="middle"
            dominantBaseline="central"
            className="fill-gray-300"
            fontSize={ACC_CUE_SIZE}
            data-testid="staff-accidental"
            data-dodged="true"
          >
            {accidentalGlyph(note.accidental)}
          </text>
        ))}
      {selected && (
        <circle
          cx={headX}
          cy={y}
          r={9}
          fill="none"
          stroke="#FBBF24"
          strokeWidth={1.5}
          data-testid="staff-halo"
        />
      )}
      {/* RV color rule: only ROOT notes carry their pitch-class color; every
        other note draws white so the roots pop and the cell reads as
        shape-around-a-root. `is_root` is computed by the Rust core. */}
      <ellipse
        cx={headX}
        cy={y}
        rx={HEAD_RX}
        ry={HEAD_RY}
        fill={
          note.is_root ? colorForPitchClass(note.midi % 12) : RV_NON_ROOT_FILL
        }
        fillOpacity={ghostSteps === 0 ? 1 : 0.7}
        data-testid="staff-dot"
      />
      {/* Half notes read as a ring: an inner void, color untouched. */}
      {showRhythms && note.duration_beats >= 2 && note.duration_beats < 4 && (
        <ellipse cx={headX} cy={y} rx={2.6} ry={1.8} fill="#111827" />
      )}
      {wantsStem && (
        <line
          x1={stemX}
          x2={stemX}
          y1={y}
          y2={stemY2}
          stroke="#9CA3AF"
          strokeWidth={1.2}
          data-testid="staff-stem"
        />
      )}
      {wantsFlag && (
        <path
          d={`M ${stemX} ${stemY2} q 7 ${stemUp ? 3 : -3} 5 ${stemUp ? 10 : -10}`}
          stroke="#9CA3AF"
          strokeWidth={1.2}
          fill="none"
          data-testid="staff-flag"
        />
      )}
    </g>
  );
}

export default function CellStaff({
  staff,
  defaultShowRhythms,
  onEditNote,
  onUndo,
  canUndo,
}: {
  staff: CellStaffViewDto;
  /** Test/override hook; real usage reads the persisted device preference. */
  defaultShowRhythms?: boolean;
  /** When present, dots become tappable/draggable (#292 slice 3). The staff
   * only ever emits semantic gestures — never pitches. */
  onEditNote?: (index: number, edit: NoteEdit) => void;
  onUndo?: () => void;
  canUndo?: boolean;
}) {
  const [showRhythms, setShowRhythms] = useState(
    defaultShowRhythms ?? readRhythmPref(),
  );
  const [selected, setSelected] = useState<number | null>(null);
  const [ghostSteps, setGhostSteps] = useState(0);
  // The staff changed under us (chip, undo, new rep): a kept selection would
  // silently retarget whatever note now holds that index — drop it.
  useEffect(() => {
    setSelected(null);
    setGhostSteps(0);
    drag.current = null;
  }, [staff]);
  const svgRef = useRef<SVGSVGElement | null>(null);
  const drag = useRef<{ index: number; startY: number } | null>(null);
  const cleanupDrag = useRef<(() => void) | null>(null);
  /** Current viewBox height — drag math reads it via ref so window-level
   * pointer handlers never close over a stale value. */
  const vbHeightRef = useRef(DEFAULT_MAX_Y - DEFAULT_MIN_Y);
  // Unmount mid-drag must not leave window listeners firing stale closures.
  useEffect(() => () => cleanupDrag.current?.(), []);

  /** px per diatonic step at the rendered size (viewBox scaling). */
  const stepPx = (vbHeight: number) => {
    const rect = svgRef.current?.getBoundingClientRect();
    return rect && rect.height > 0 ? (rect.height / vbHeight) * STEP : STEP;
  };
  const dragSteps = (e: { clientY: number }) => {
    if (!drag.current || !Number.isFinite(e.clientY)) {
      return 0;
    }
    return Math.round(
      (drag.current.startY - e.clientY) / stepPx(vbHeightRef.current),
    );
  };

  const beginDrag = (index: number) => (e: React.PointerEvent) => {
    if (!onEditNote) return;
    setSelected(index);
    drag.current = { index, startY: e.clientY };
    setGhostSteps(0);
    const move = (ev: PointerEvent) => setGhostSteps(dragSteps(ev));
    const finish = (ev: PointerEvent | null) => {
      const by = ev ? dragSteps(ev) : 0;
      const idx = drag.current?.index;
      drag.current = null;
      setGhostSteps(0);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      cleanupDrag.current = null;
      if (ev && idx !== undefined && Number.isFinite(by) && by !== 0) {
        onEditNote(idx, { kind: "staff_steps", by });
      }
    };
    const up = (ev: PointerEvent) => finish(ev);
    const cancel = () => finish(null); // touch-scroll etc.: no edit, no ghost
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    cleanupDrag.current = () => finish(null);
  };

  const act = (edit: NoteEdit) => {
    if (selected !== null && onEditNote) {
      onEditNote(selected, edit);
    }
  };
  const toggleRhythms = () => {
    const next = !showRhythms;
    setShowRhythms(next);
    try {
      window.localStorage.setItem(RHYTHM_PREF_KEY, next ? "1" : "0");
    } catch {
      // Preference is a nicety; rendering must never depend on storage.
    }
  };
  const beatsPerMeasure = Math.max(1, staff.beats_per_measure);
  const totalMeasures = Math.max(
    1,
    Math.ceil(staff.total_beats / beatsPerMeasure),
  );
  const systems = Math.ceil(totalMeasures / MEASURES_PER_SYSTEM);
  // #445-1: the first barline waits for the key signature — the left pad
  // grows with the signature instead of letting glyphs spill past it.
  const sigCount = Math.min(Math.abs(staff.fifths), 7);
  const leftPad = CLEF_ZONE + sigCount * KEYSIG_STEP + KEYSIG_GAP;
  const innerWidth = WIDTH - leftPad - RIGHT_PAD;
  /** Constant measure width — a partial final system ends mid-line, like a
   * printed final system, instead of stretching to a different scale. */
  const measureWidth = innerWidth / MEASURES_PER_SYSTEM;
  const barX = (measureInSystem: number) =>
    leftPad + measureInSystem * measureWidth;
  /** #445-1: ONE grid. Notes are placed inside their own measure's span
   * (barline + inset .. next barline - inset) — no second scale to drift
   * against the barlines. */
  const xFor = (beat: number) => {
    const measure = Math.floor(beat / beatsPerMeasure);
    const beatIn = beat - measure * beatsPerMeasure;
    const span = measureWidth - NOTE_INSET_L - NOTE_INSET_R;
    return (
      barX(measure % MEASURES_PER_SYSTEM) +
      NOTE_INSET_L +
      (beatIn / beatsPerMeasure) * span
    );
  };
  const systemOf = (beat: number) =>
    Math.floor(beat / beatsPerMeasure / MEASURES_PER_SYSTEM);

  const visible = staff.notes.map((n, index) => ({ n, index }));
  // #349 T2a: notes sharing a beat draw as a vertical stack for free (same
  // x). The one engraving rule stacks need: when two simultaneous notes
  // collide — adjacent steps (a second) or the SAME step with different
  // pitches (a chromatic second like C + C#) — the upper notehead shifts
  // right so both stay readable. (A cluster of consecutive seconds, e.g.
  // add9's C-D-E, shifts both uppers to one offset column; acceptable until
  // the T2 vocabulary actually deals clusters.)
  const secondOffset = (note: CellStaffNoteDto) =>
    visible.some(
      ({ n: other }) =>
        other !== note &&
        other.start_beat === note.start_beat &&
        (note.step - other.step === 1 ||
          (note.step === other.step && note.midi > other.midi)),
    )
      ? 9
      : 0;
  // #471-1: accidental collision pass — pure geometry over the layout the
  // notes already have. An accidental whose in-line box would land within
  // ACC_CLEAR of any same-system notehead box (BOTH axes — vertical
  // separation keeps a glyph honestly in-line) hoists above its column.
  // Sparse layouts take the `undefined` path everywhere: byte-identical.
  const placed = visible.map(({ n, index }) => {
    const colX = xFor(n.start_beat);
    return {
      n,
      index,
      colX,
      headX: colX + secondOffset(n),
      y: yFor(n.step),
      sys: systemOf(n.start_beat),
    };
  });
  const collidesInline = (a: (typeof placed)[number]) =>
    placed.some((p) => {
      if (p.sys !== a.sys) return false;
      const accL = a.colX - ACC_ANCHOR - ACC_W;
      const accR = a.colX - ACC_ANCHOR;
      const gapX = Math.max(
        p.headX - HEAD_RX - accR,
        accL - (p.headX + HEAD_RX),
      );
      const gapY = Math.max(
        p.y - HEAD_RY - (a.y + ACC_HALF_H),
        a.y - ACC_HALF_H - (p.y + HEAD_RY),
      );
      return gapX < ACC_CLEAR && gapY < ACC_CLEAR;
    });
  // Topmost head of each occupied column — hoisted glyphs rise above it.
  const colTop = new Map<string, number>();
  for (const p of placed) {
    const key = `${p.sys}:${p.n.start_beat}`;
    const cur = colTop.get(key);
    if (cur === undefined || p.y < cur) colTop.set(key, p.y);
  }
  /** note index → hoisted glyph center y (undefined = in-line). */
  const accDodge = new Map<number, number>();
  {
    // Round-1 review MF1: a hoist must VERIFY its landing space — a
    // stacked-second's +9 offset head one column over can occupy the first
    // candidate slot (measured: −2.2px into the foreign head). The cue box
    // rises by ACC_DODGE_STACK until it clears EVERY same-system head.
    const cueClear = (sys: number, colX: number, y: number) =>
      placed.every((p) => {
        if (p.sys !== sys) return true;
        const gapX = Math.max(
          p.headX - HEAD_RX - (colX + ACC_CUE_HALF_W),
          colX - ACC_CUE_HALF_W - (p.headX + HEAD_RX),
        );
        const gapY = Math.max(
          p.y - HEAD_RY - (y + ACC_CUE_HALF_H),
          y - ACC_CUE_HALF_H - (p.y + HEAD_RY),
        );
        return gapX >= ACC_CLEAR || gapY >= ACC_CLEAR;
      });
    // Per-column candidate slots keep stacked glyphs apart even after a
    // rise: the next glyph starts one ACC_DODGE_STACK above the last.
    const nextY = new Map<string, number>();
    const hoisted = placed
      .filter((p) => p.n.accidental !== null && collidesInline(p))
      // Higher note's glyph sits nearest the heads when a column stacks.
      .sort((a, b) => b.n.step - a.n.step);
    for (const p of hoisted) {
      const key = `${p.sys}:${p.n.start_beat}`;
      let y = nextY.get(key) ?? (colTop.get(key) ?? p.y) - ACC_DODGE_RISE;
      // Terminates: y strictly decreases and heads are bounded above.
      while (!cueClear(p.sys, p.colX, y)) y -= ACC_DODGE_STACK;
      accDodge.set(p.index, y);
      nextY.set(key, y - ACC_DODGE_STACK);
    }
  }
  const [rawMinY, rawHeight] = viewBoxFor(visible.map(({ n }) => n.step));
  // No-vanish rule extends to hoisted glyphs: the band grows upward to
  // cover their ink (+ACC_CLEAR of margin), for every system's stride.
  const accTop = Math.min(
    rawMinY,
    ...[...accDodge.values()].map((y) => y - ACC_CUE_HALF_H - ACC_CLEAR),
  );
  const vbMinY = accTop;
  const bandHeight = rawHeight + (rawMinY - accTop);
  const systemStride = bandHeight + SYSTEM_GAP;
  const vbHeight = systems * bandHeight + (systems - 1) * SYSTEM_GAP;
  vbHeightRef.current = vbHeight;
  const sigSteps =
    staff.fifths > 0
      ? SHARP_STEPS.slice(0, Math.min(staff.fifths, 7))
      : FLAT_STEPS.slice(0, Math.min(-staff.fifths, 7));
  const sigGlyph = staff.fifths > 0 ? "♯" : "♭";

  return (
    <div className="flex flex-col gap-1" data-testid="cell-staff">
      <svg
        ref={svgRef}
        viewBox={`0 ${vbMinY} ${WIDTH} ${vbHeight}`}
        className="h-auto w-full"
        role="img"
        aria-label="Cell staff"
      >
        {Array.from({ length: systems }, (_, k) => {
          const measuresInSystem = Math.min(
            MEASURES_PER_SYSTEM,
            totalMeasures - k * MEASURES_PER_SYSTEM,
          );
          const isLastSystem = k === systems - 1;
          return (
            // Every system is a full printed line: staff, clef, key
            // signature, its own barlines — translated down by its stride.
            <g
              key={k}
              transform={`translate(0, ${k * systemStride})`}
              data-testid={`staff-system-${k}`}
            >
              {[0, 2, 4, 6, 8].map((s) => (
                <line
                  key={s}
                  x1={8}
                  x2={WIDTH - 8}
                  y1={yFor(s)}
                  y2={yFor(s)}
                  stroke="#6B7280"
                  strokeWidth={1}
                />
              ))}
              {/* Treble clef */}
              <text
                x={10}
                y={yFor(4)}
                dominantBaseline="central"
                fontSize={52}
                className="fill-gray-400 select-none"
              >
                𝄞
              </text>
              {/* Key signature */}
              {sigSteps.map((s, i) => (
                <text
                  key={`${s}-${i}`}
                  x={CLEF_ZONE + i * KEYSIG_STEP}
                  y={yFor(s)}
                  dominantBaseline="central"
                  fontSize={14}
                  className="fill-gray-300"
                  data-testid="staff-signature"
                >
                  {sigGlyph}
                </text>
              ))}
              {/* Barlines — one shared grid; notes live INSIDE it. */}
              {Array.from({ length: measuresInSystem + 1 }, (_, i) => {
                const x = barX(i);
                return (
                  <line
                    key={i}
                    x1={x}
                    x2={x}
                    y1={yFor(8)}
                    y2={yFor(0)}
                    stroke="#4B5563"
                    data-testid={`staff-barline-${k}-${i}`}
                    strokeWidth={isLastSystem && i === measuresInSystem ? 3 : 1}
                  />
                );
              })}
              {visible
                .filter(({ n }) => systemOf(n.start_beat) === k)
                .map(({ n, index }) => (
                  <Dot
                    key={`${n.midi}-${n.start_beat}`}
                    note={n}
                    x={xFor(n.start_beat)}
                    headOffset={secondOffset(n)}
                    accDodgeY={accDodge.get(index)}
                    showRhythms={showRhythms}
                    selected={selected === index}
                    ghostSteps={drag.current?.index === index ? ghostSteps : 0}
                    onPointerDown={onEditNote ? beginDrag(index) : undefined}
                  />
                ))}
            </g>
          );
        })}
      </svg>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          {onEditNote && selected !== null && (
            <div className="flex gap-1.5" data-testid="edit-actions">
              <button
                type="button"
                onClick={() => act({ kind: "octaves", by: 1 })}
                data-testid="edit-octave-up"
                className="rounded bg-gray-700 px-2 py-0.5 text-xs text-gray-200 hover:bg-gray-600"
              >
                8va ↑
              </button>
              <button
                type="button"
                onClick={() => act({ kind: "octaves", by: -1 })}
                data-testid="edit-octave-down"
                className="rounded bg-gray-700 px-2 py-0.5 text-xs text-gray-200 hover:bg-gray-600"
              >
                8va ↓
              </button>
              <button
                type="button"
                onClick={() => act({ kind: "semitones", by: 1 })}
                data-testid="edit-sharp"
                className="rounded bg-gray-700 px-2 py-0.5 text-xs text-gray-200 hover:bg-gray-600"
              >
                ♯
              </button>
              <button
                type="button"
                onClick={() => act({ kind: "semitones", by: -1 })}
                data-testid="edit-flat"
                className="rounded bg-gray-700 px-2 py-0.5 text-xs text-gray-200 hover:bg-gray-600"
              >
                ♭
              </button>
              <button
                type="button"
                onClick={() => {
                  act({ kind: "remove" });
                  setSelected(null);
                }}
                data-testid="edit-remove"
                className="rounded bg-gray-700 px-2 py-0.5 text-xs text-red-300 hover:bg-gray-600"
              >
                ✕
              </button>
            </div>
          )}
          {onUndo && canUndo && (
            <button
              type="button"
              onClick={onUndo}
              data-testid="edit-undo"
              className="rounded px-2 py-0.5 text-xs text-gray-400 hover:text-gray-200"
            >
              ↩ undo
            </button>
          )}
        </div>
        <button
          type="button"
          onClick={toggleRhythms}
          data-testid="rhythm-toggle"
          aria-pressed={showRhythms}
          className={`rounded px-2 py-0.5 text-xs ${
            showRhythms
              ? "bg-gray-600 text-gray-100"
              : "text-gray-500 hover:text-gray-300"
          }`}
        >
          ♪ rhythms
        </button>
      </div>
    </div>
  );
}
