import { usePracticeStore } from "../stores/practiceStore";
import type { NoteVerdictKind } from "../stores/practiceStore";

const DOT_COLOR: Record<NoteVerdictKind, string> = {
  hit: "#34d399", // emerald — clean hit
  near: "#fbbf24", // amber — right note, rough tuning (or a semitone off)
  missed: "#f87171", // red — wrong or skipped
};

/**
 * #337 S2 — the live "it's listening" tally for score practice: running
 * hit/near/missed counts plus the last few verdicts as colored dots. Renders
 * nothing until the follower has judged at least one note, so free play and
 * a not-yet-locked follower stay clean (silence > lies).
 */
export default function VerdictStrip() {
  const verdicts = usePracticeStore((s) => s.noteVerdicts);
  const total = verdicts.hit + verdicts.near + verdicts.missed;
  if (total === 0) return null;

  return (
    <div
      className="flex items-center gap-3 rounded-md bg-gray-900/60 px-3 py-1.5 text-sm"
      data-testid="verdict-strip"
    >
      <span className="text-emerald-400" data-testid="verdict-hit">
        ✓ {verdicts.hit}
      </span>
      <span className="text-amber-400" data-testid="verdict-near">
        ~ {verdicts.near}
      </span>
      <span className="text-red-400" data-testid="verdict-missed">
        ✗ {verdicts.missed}
      </span>
      <span className="flex items-center gap-1" aria-hidden="true">
        {verdicts.recent.map((v, i) => (
          <span
            key={i}
            data-testid="verdict-dot"
            className="inline-block h-2 w-2 rounded-full"
            style={{ backgroundColor: DOT_COLOR[v] }}
          />
        ))}
      </span>
    </div>
  );
}
