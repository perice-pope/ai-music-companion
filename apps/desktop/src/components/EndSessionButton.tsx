import { useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";

/**
 * Small header button that ends the active session.
 *
 * While the `end_practice_session` command is in flight (recap
 * generation takes up to 30s for real LLM) the button shows a quiet
 * "Ending…" state and is disabled. If the command throws, the store's
 * error handler navigates to the recap screen with `recapError` set
 * so the fallback copy renders — this component just kicks off the
 * call and surrenders control.
 */
export default function EndSessionButton() {
  const status = usePracticeStore((s) => s.status);
  const endSession = usePracticeStore((s) => s.endSession);
  const [localError, setLocalError] = useState<string | null>(null);

  const disabled = status !== "listening";
  const busy = status === "ending";

  const onClick = async () => {
    setLocalError(null);
    try {
      await endSession();
    } catch (err) {
      // The store already handles the happy-error path (navigates to
      // recap with `recapError`). We capture anything it re-throws
      // here just in case — e.g. if someone passes a non-rejection
      // promise.
      setLocalError(String(err));
    }
  };

  return (
    <div className="flex items-center gap-2">
      {localError && (
        <span className="text-xs text-red-400" role="alert">
          {localError}
        </span>
      )}
      <button
        type="button"
        onClick={onClick}
        disabled={disabled}
        data-testid="end-session-button"
        className={`rounded border px-3 py-1 text-sm transition-colors
          ${
            disabled
              ? "cursor-not-allowed border-gray-700 bg-gray-800 text-gray-500"
              : "border-gray-500 bg-gray-700 text-gray-100 hover:bg-gray-600"
          }`}
      >
        {busy ? "Ending…" : "End Session"}
      </button>
    </div>
  );
}
