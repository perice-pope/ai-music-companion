import { usePracticeStore } from "../stores/practiceStore";
import ScoreView from "./ScoreView";

/** Percent formatting for drill accuracies. */
function pct(x: number): string {
  return `${Math.round(x * 100)}%`;
}

/**
 * The guided lesson surface (#254): shows the current drill's sheet music with
 * its label/tempo/difficulty, lets the player submit what they just played for
 * grading, and renders the final recap. All lesson logic lives in the Rust
 * core — this panel only steps the state machine.
 */
export default function LessonPanel() {
  const drill = usePracticeStore((s) => s.lessonDrill);
  const score = usePracticeStore((s) => s.lessonScore);
  const recap = usePracticeStore((s) => s.lessonRecap);
  const submitDrill = usePracticeStore((s) => s.submitDrill);
  const endLesson = usePracticeStore((s) => s.endLesson);
  const clearLessonRecap = usePracticeStore((s) => s.clearLessonRecap);

  if (recap) {
    return (
      <div
        className="mx-auto w-full max-w-2xl rounded-lg border border-indigo-700 bg-indigo-950/40 p-6"
        data-testid="lesson-recap"
      >
        <h2 className="text-lg font-semibold text-indigo-100">
          Lesson complete 🎉
        </h2>
        <ul className="mt-4 space-y-2">
          {recap.drill_labels.map((label, i) => (
            <li
              key={label + i}
              className="flex items-baseline justify-between gap-4 text-sm"
            >
              <span className="text-indigo-200/90">{label}</span>
              <span className="font-semibold text-indigo-100">
                {pct(recap.drill_accuracies[i] ?? 0)}
              </span>
            </li>
          ))}
        </ul>
        <p className="mt-4 text-sm text-indigo-200/80">
          Difficulty: step {recap.start_difficulty} → step{" "}
          {recap.end_difficulty}
          {recap.end_difficulty > recap.start_difficulty
            ? " — leveled up!"
            : ""}
        </p>
        <button
          type="button"
          onClick={clearLessonRecap}
          data-testid="lesson-recap-done"
          className="mt-6 rounded bg-indigo-600 px-4 py-2 text-sm font-semibold text-white hover:bg-indigo-500"
        >
          Done
        </button>
      </div>
    );
  }

  if (!drill) {
    return null;
  }

  return (
    <div className="flex min-h-0 w-full flex-1 flex-col gap-3" data-testid="lesson-panel">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="text-xs font-semibold uppercase tracking-wider text-indigo-300/80">
            Lesson · drill {drill.index + 1} of {drill.drill_count} · step{" "}
            {drill.difficulty}
          </p>
          <p className="text-sm font-medium text-gray-100">{drill.label}</p>
        </div>
        <div className="flex items-center gap-2">
          {score && (
            <span
              className="rounded bg-indigo-900/60 px-2 py-1 text-xs font-semibold text-indigo-200"
              data-testid="lesson-last-score"
            >
              last drill: {pct(score.accuracy)}
            </span>
          )}
          <button
            type="button"
            onClick={() => void submitDrill()}
            data-testid="lesson-submit"
            className="rounded bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500"
          >
            I played it — grade me
          </button>
          <button
            type="button"
            onClick={() => void endLesson()}
            data-testid="lesson-end"
            className="rounded bg-gray-700 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-600"
          >
            End lesson
          </button>
        </div>
      </div>
      <div className="min-h-0 flex-1" data-testid="lesson-score-pane">
        <ScoreView musicXml={drill.music_xml} cursorPosition={null} />
      </div>
    </div>
  );
}
