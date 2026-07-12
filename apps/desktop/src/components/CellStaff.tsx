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
 * music theory). Shows a 4-measure window with a minimal pager.
 */

/** Vertical gap between staff lines (px in the SVG's own units). */
const LINE_GAP = 10;
/** One diatonic step = half a line gap. */
const STEP = LINE_GAP / 2;
/** y of the bottom staff line (step 0); higher steps go UP (smaller y). */
const BOTTOM_Y = 5 * LINE_GAP;
/** Measures per page — the founder's "2 or 4 at a time". */
export const MEASURES_PER_PAGE = 4;

const WIDTH = 640;
const LEFT_PAD = 56; // room for clef + key signature
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
  showRhythms,
  selected,
  ghostSteps,
  onPointerDown,
}: {
  note: CellStaffNoteDto;
  x: number;
  showRhythms: boolean;
  selected: boolean;
  /** Live drag preview: diatonic steps the ghost has moved (0 = at rest). */
  ghostSteps: number;
  onPointerDown?: (e: React.PointerEvent) => void;
}) {
  const y = yFor(note.step + ghostSteps);
  // Rhythm layer (#292 slice 2): stems/flags are drawn ON the same dot at the
  // same position — the layer NEVER moves a notehead. Whole notes stay bare.
  const stemUp = note.step < 4;
  const stemX = stemUp ? x + 5 : x - 5;
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
          x1={x - 9}
          x2={x + 9}
          y1={yFor(s)}
          y2={yFor(s)}
          stroke="#9CA3AF"
          strokeWidth={1}
        />
      ))}
      {note.accidental !== null && (
        <text
          x={x - 11}
          y={y}
          textAnchor="end"
          dominantBaseline="central"
          className="fill-gray-300"
          fontSize={13}
          data-testid="staff-accidental"
        >
          {accidentalGlyph(note.accidental)}
        </text>
      )}
      {selected && (
        <circle
          cx={x}
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
        cx={x}
        cy={y}
        rx={5.5}
        ry={4.2}
        fill={note.is_root ? colorForPitchClass(note.midi % 12) : RV_NON_ROOT_FILL}
        fillOpacity={ghostSteps === 0 ? 1 : 0.7}
        data-testid="staff-dot"
      />
      {/* Half notes read as a ring: an inner void, color untouched. */}
      {showRhythms && note.duration_beats >= 2 && note.duration_beats < 4 && (
        <ellipse cx={x} cy={y} rx={2.6} ry={1.8} fill="#111827" />
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
  const [page, setPage] = useState(0);
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
  const pages = Math.ceil(totalMeasures / MEASURES_PER_PAGE);
  const current = Math.min(page, pages - 1);
  const windowStart = current * MEASURES_PER_PAGE * beatsPerMeasure;
  const windowMeasures = Math.min(
    MEASURES_PER_PAGE,
    totalMeasures - current * MEASURES_PER_PAGE,
  );
  const windowBeats = windowMeasures * beatsPerMeasure;
  const innerWidth = WIDTH - LEFT_PAD - RIGHT_PAD;
  const xFor = (beat: number) =>
    LEFT_PAD + 14 + ((beat - windowStart) / windowBeats) * (innerWidth - 20);

  const visible = staff.notes
    .map((n, index) => ({ n, index }))
    .filter(
      ({ n }) =>
        n.start_beat >= windowStart && n.start_beat < windowStart + windowBeats,
    );
  // #349 T2a: notes sharing a beat draw as a vertical stack for free (same
  // x). The one engraving rule stacks need: when two simultaneous notes sit
  // a SECOND apart (adjacent steps), the upper notehead shifts right so
  // both stay readable instead of overlapping.
  const secondOffset = (note: CellStaffNoteDto) =>
    visible.some(
      ({ n: other }) =>
        other !== note &&
        other.start_beat === note.start_beat &&
        note.step - other.step === 1,
    )
      ? 9
      : 0;
  const [vbMinY, vbHeight] = viewBoxFor(visible.map(({ n }) => n.step));
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
            x={38 + i * 8}
            y={yFor(s)}
            dominantBaseline="central"
            fontSize={14}
            className="fill-gray-300"
            data-testid="staff-signature"
          >
            {sigGlyph}
          </text>
        ))}
        {/* Barlines */}
        {Array.from({ length: windowMeasures + 1 }, (_, i) => {
          const x =
            LEFT_PAD +
            (i * (innerWidth - 6)) / windowMeasures +
            (i === 0 ? 0 : 6);
          return (
            <line
              key={i}
              x1={x}
              x2={x}
              y1={yFor(8)}
              y2={yFor(0)}
              stroke="#4B5563"
              strokeWidth={i === windowMeasures && current === pages - 1 ? 3 : 1}
            />
          );
        })}
        {visible.map(({ n, index }) => (
          <Dot
            key={`${n.midi}-${n.start_beat}`}
            note={n}
            x={xFor(n.start_beat) + secondOffset(n)}
            showRhythms={showRhythms}
            selected={selected === index}
            ghostSteps={drag.current?.index === index ? ghostSteps : 0}
            onPointerDown={onEditNote ? beginDrag(index) : undefined}
          />
        ))}
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
      {pages > 1 && (
        <div
          className="flex items-center justify-center gap-3 text-xs text-gray-400"
          data-testid="staff-pager"
        >
          <button
            type="button"
            onClick={() => {
              setPage(Math.max(0, current - 1));
              setSelected(null);
            }}
            disabled={current === 0}
            className="rounded px-2 py-0.5 hover:bg-gray-700 disabled:opacity-30"
            aria-label="Previous measures"
          >
            ‹
          </button>
          <span>
            {current + 1} / {pages}
          </span>
          <button
            type="button"
            onClick={() => {
              setPage(Math.min(pages - 1, current + 1));
              setSelected(null);
            }}
            disabled={current === pages - 1}
            className="rounded px-2 py-0.5 hover:bg-gray-700 disabled:opacity-30"
            aria-label="Next measures"
          >
            ›
          </button>
        </div>
      )}
    </div>
  );
}
