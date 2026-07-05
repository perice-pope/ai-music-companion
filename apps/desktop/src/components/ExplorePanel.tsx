import { usePracticeStore } from "../stores/practiceStore";
import { colorForPitchClass, nameForPitchClass } from "../lib/rvColors";
import CellStaff from "./CellStaff";

/**
 * The free-play exploration surface (#255): an RV variation seeded from the
 * sound the player was just in, with ≤3 mutation chips underneath. The
 * frontend performs no theory — every chip echoes back the exact delta the
 * backend attached to it.
 */
export default function ExplorePanel() {
  const explore = usePracticeStore((s) => s.explore);
  const applyChip = usePracticeStore((s) => s.applyChip);
  const endExplore = usePracticeStore((s) => s.endExplore);

  if (!explore) {
    return null;
  }

  return (
    <div
      className="flex min-h-0 w-full flex-1 flex-col gap-3"
      data-testid="explore-panel"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div
          className="border-l-4 pl-3"
          style={{
            borderColor: colorForPitchClass(explore.root_pitch_classes[0] ?? 0),
          }}
        >
          <p className="text-xs font-semibold uppercase tracking-wider text-amber-300/80">
            Exploring
          </p>
          <p className="text-sm font-medium text-gray-100">{explore.label}</p>
        </div>
        <button
          type="button"
          onClick={() => void endExplore()}
          data-testid="explore-end"
          className="rounded bg-gray-700 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-600"
        >
          Back to listening
        </button>
      </div>

      {/* RV colored cells: the roots in play order (#278). */}
      <div
        className="flex flex-wrap items-center gap-1.5"
        data-testid="explore-root-cells"
      >
        {explore.root_pitch_classes.map((pc, i) => (
          <span
            key={`${pc}-${i}`}
            className="inline-flex h-8 min-w-8 items-center justify-center rounded-md px-1.5 text-sm font-bold text-gray-900 shadow"
            style={{ backgroundColor: colorForPitchClass(pc) }}
          >
            {nameForPitchClass(pc)}
          </span>
        ))}
      </div>

      {/* The RV dot staff (#292): stemless colored noteheads, no white page. */}
      <div className="min-h-0 flex-1">
        <CellStaff staff={explore.staff} />
      </div>

      {/* The mutation chips — tapping one is the whole conversation. */}
      <div className="flex flex-wrap gap-2" data-testid="explore-chips">
        {explore.chips.map((chip) => (
          <button
            key={chip.label}
            type="button"
            onClick={() => void applyChip(chip.delta)}
            data-testid={`chip-${chip.delta.kind}`}
            className="rounded-full bg-indigo-600/90 px-4 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500"
          >
            {chip.label}
          </button>
        ))}
      </div>
    </div>
  );
}
