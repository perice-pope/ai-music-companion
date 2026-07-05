import { useState } from "react";

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
import { colorForPitchClass } from "../lib/rvColors";

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
}: {
  note: CellStaffNoteDto;
  x: number;
  showRhythms: boolean;
}) {
  const y = yFor(note.step);
  // Rhythm layer (#292 slice 2): stems/flags are drawn ON the same dot at the
  // same position — the layer NEVER moves a notehead. Whole notes stay bare.
  const stemUp = note.step < 4;
  const stemX = stemUp ? x + 5 : x - 5;
  const stemY2 = stemUp ? y - 3.2 * LINE_GAP : y + 3.2 * LINE_GAP;
  const wantsStem = showRhythms && note.duration_beats < 4;
  const wantsFlag = showRhythms && note.duration_beats < 1;
  return (
    <g data-testid={`staff-note-${note.midi}-${note.start_beat}`}>
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
      <ellipse
        cx={x}
        cy={y}
        rx={5.5}
        ry={4.2}
        fill={colorForPitchClass(note.midi % 12)}
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
}: {
  staff: CellStaffViewDto;
  /** Test/override hook; real usage reads the persisted device preference. */
  defaultShowRhythms?: boolean;
}) {
  const [page, setPage] = useState(0);
  const [showRhythms, setShowRhythms] = useState(
    defaultShowRhythms ?? readRhythmPref(),
  );
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

  const visible = staff.notes.filter(
    (n) => n.start_beat >= windowStart && n.start_beat < windowStart + windowBeats,
  );
  const sigSteps =
    staff.fifths > 0
      ? SHARP_STEPS.slice(0, Math.min(staff.fifths, 7))
      : FLAT_STEPS.slice(0, Math.min(-staff.fifths, 7));
  const sigGlyph = staff.fifths > 0 ? "♯" : "♭";

  return (
    <div className="flex flex-col gap-1" data-testid="cell-staff">
      <svg
        viewBox={`0 -${2 * LINE_GAP} ${WIDTH} ${HEIGHT + LINE_GAP}`}
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
        {visible.map((n) => (
          <Dot
            key={`${n.midi}-${n.start_beat}`}
            note={n}
            x={xFor(n.start_beat)}
            showRhythms={showRhythms}
          />
        ))}
      </svg>
      <div className="flex justify-end">
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
            onClick={() => setPage(Math.max(0, current - 1))}
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
            onClick={() => setPage(Math.min(pages - 1, current + 1))}
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
