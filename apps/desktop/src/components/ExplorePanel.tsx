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
  const notice = usePracticeStore((s) => s.exploreNotice);
  const applyChip = usePracticeStore((s) => s.applyChip);
  const editNote = usePracticeStore((s) => s.editExploreNote);
  const undoEdit = usePracticeStore((s) => s.undoExploreEdit);
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
            {/* Backend-spelled (#335): flat keys say Db, matching the
              staff's signature — never a contradicting C#. */}
            {explore.root_names?.[i] ?? nameForPitchClass(pc)}
          </span>
        ))}
      </div>

      {/* The RV dot staff (#292): stemless colored noteheads, tap/drag to
          edit the cell — one gesture fixes the note in every key. */}
      <div className="min-h-0 flex-1">
        <CellStaff
          staff={explore.staff}
          onEditNote={(index, edit) => void editNote(index, edit)}
          onUndo={() => void undoEdit()}
          canUndo={explore.can_undo}
        />
      </div>

      {notice && (
        <p
          className="text-xs italic text-gray-400"
          data-testid="explore-notice"
          role="status"
        >
          {notice}
        </p>
      )}
      {/* The mutation chips — tapping one is the whole conversation.
          #445-4: the row is STABLE — five chips, fixed identity; a chip
          that can't act right now dims in place instead of vanishing. */}
      <div className="flex flex-wrap gap-2" data-testid="explore-chips">
        {explore.chips.map((chip) => (
          <button
            key={chip.label}
            type="button"
            onClick={() => void applyChip(chip.delta)}
            disabled={chip.enabled === false}
            data-testid={`chip-${chip.delta.kind}${
              chip.delta.kind === "bump_difficulty"
                ? chip.delta.by > 0
                  ? "-up"
                  : "-down"
                : ""
            }`}
            className="rounded-full bg-indigo-600/90 px-4 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-indigo-600/90"
          >
            {chip.label}
          </button>
        ))}
      </div>
    </div>
  );
}
