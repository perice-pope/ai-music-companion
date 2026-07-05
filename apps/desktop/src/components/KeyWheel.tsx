import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { KeyCellDto, WheelViewDto, WheelTrend } from "../types/brain";
import { colorForPitchClass, nameForPitchClass } from "../lib/rvColors";

/** Circle-of-fifths layout order (pitch classes clockwise from 12 o'clock). */
const FIFTHS_ORDER: number[] = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5];

/** Fill opacity per state: owned glows, learning shows, none rests. */
const STATE_OPACITY: Record<string, number> = {
  owned: 1,
  learning: 0.45,
  none: 0.1,
};

function trendCopy(label: string, t: WheelTrend): string | null {
  switch (t) {
    case "improving":
      return `${label}: improving`;
    case "slipping":
      return `${label}: needs love`;
    case "steady":
      return `${label}: steady`;
    default:
      return null; // unknown → say nothing rather than guess
  }
}

/** One donut segment of the wheel. */
function Segment({
  cell,
  index,
  onSelect,
  selected,
}: {
  cell: KeyCellDto;
  index: number;
  onSelect: (cell: KeyCellDto) => void;
  selected: boolean;
}) {
  // 12 segments, 30° each, starting at 12 o'clock; small gap between them.
  const start = (index * 30 - 90 + 1.5) * (Math.PI / 180);
  const end = ((index + 1) * 30 - 90 - 1.5) * (Math.PI / 180);
  const [cx, cy, rOuter, rInner] = [100, 100, 90, 58];
  const p = (r: number, a: number) => `${cx + r * Math.cos(a)},${cy + r * Math.sin(a)}`;
  const d = [
    `M ${p(rOuter, start)}`,
    `A ${rOuter} ${rOuter} 0 0 1 ${p(rOuter, end)}`,
    `L ${p(rInner, end)}`,
    `A ${rInner} ${rInner} 0 0 0 ${p(rInner, start)}`,
    "Z",
  ].join(" ");
  const mid = (start + end) / 2;
  const rLabel = (rOuter + rInner) / 2;

  return (
    <g
      onClick={() => onSelect(cell)}
      data-testid={`wheel-cell-${cell.tonic}`}
      data-state={cell.state}
      className="cursor-pointer"
    >
      <path
        d={d}
        fill={colorForPitchClass(cell.tonic)}
        fillOpacity={STATE_OPACITY[cell.state] ?? 0.1}
        stroke={selected ? "#FFFFFF" : "#1F2937"}
        strokeWidth={selected ? 2 : 1}
      />
      <text
        x={cx + rLabel * Math.cos(mid)}
        y={cy + rLabel * Math.sin(mid)}
        textAnchor="middle"
        dominantBaseline="central"
        className="select-none fill-gray-100 text-[11px] font-bold"
      >
        {nameForPitchClass(cell.tonic)}
      </text>
    </g>
  );
}

/**
 * The 12-key mastery wheel (#256): a read-only snapshot of the Learner Model.
 * Keys light up in the RV brand colors as they're owned; tap a key for its
 * detail. Fetched on mount — the backend derives everything.
 */
export default function KeyWheel() {
  const [wheel, setWheel] = useState<WheelViewDto | null>(null);
  const [selected, setSelected] = useState<KeyCellDto | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const view = await invoke<WheelViewDto>("get_mastery_wheel");
        if (!cancelled) setWheel(view);
      } catch (err) {
        console.error("get_mastery_wheel failed:", err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (!wheel) {
    return null;
  }

  const trends = [
    trendCopy("Intonation", wheel.intonation_trend),
    trendCopy("Tone", wheel.tone_trend),
  ].filter(Boolean);

  return (
    <div
      className="flex flex-col items-center gap-2"
      data-testid="key-wheel"
    >
      <p className="text-xs font-semibold uppercase tracking-wider text-gray-400">
        Your keys · {wheel.total_owned} of 12 owned
      </p>
      <svg viewBox="0 0 200 200" className="h-52 w-52" role="img" aria-label="Key mastery wheel">
        {FIFTHS_ORDER.map((pc, i) => (
          <Segment
            key={pc}
            cell={wheel.cells[pc]}
            index={i}
            onSelect={(c) => setSelected(selected?.tonic === c.tonic ? null : c)}
            selected={selected?.tonic === pc}
          />
        ))}
        <text
          x="100"
          y="100"
          textAnchor="middle"
          dominantBaseline="central"
          className="fill-gray-500 text-[10px]"
        >
          {wheel.total_owned === 0 ? "play to light up" : `${wheel.total_owned}/12`}
        </text>
      </svg>
      {trends.length > 0 && (
        <p className="text-xs text-gray-400" data-testid="wheel-trends">
          {trends.join(" · ")}
        </p>
      )}
      {selected && (
        <div
          className="w-64 rounded-md border border-gray-700 bg-gray-800/80 p-3 text-sm"
          data-testid="wheel-detail"
        >
          <p className="font-semibold text-gray-100">
            {nameForPitchClass(selected.tonic)} —{" "}
            {selected.state === "owned"
              ? "owned ✨"
              : selected.state === "learning"
                ? "learning"
                : "not explored yet"}
          </p>
          {selected.attempts > 0 ? (
            <p className="mt-1 text-gray-300">
              {selected.attempts} drills · best{" "}
              {Math.round(selected.best_accuracy * 100)}%
              {selected.scales.length > 0 && (
                <> · {selected.scales.join(", ")}</>
              )}
            </p>
          ) : (
            <p className="mt-1 text-gray-400">
              Start a lesson or explore a reveal to work this key.
            </p>
          )}
        </div>
      )}
    </div>
  );
}
