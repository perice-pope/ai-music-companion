import { useEffect, useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";
import type { CoachingSeverity, CoachingCategory } from "../types/brain";

/** Styling for severity badges. */
function severityColor(severity: CoachingSeverity): string {
  switch (severity) {
    case "encouragement":
      return "bg-green-900/50 text-green-300 border-green-700";
    case "suggestion":
      return "bg-blue-900/50 text-blue-300 border-blue-700";
    case "focus":
      return "bg-yellow-900/50 text-yellow-300 border-yellow-700";
    default:
      return "bg-gray-700/50 text-gray-300 border-gray-600";
  }
}

/** Styling for category badges. */
function categoryColor(category: CoachingCategory): string {
  switch (category) {
    case "tone":
      return "bg-purple-900/40 text-purple-300";
    case "intonation":
      return "bg-cyan-900/40 text-cyan-300";
    case "rhythm":
      return "bg-indigo-900/40 text-indigo-300";
    case "dynamics":
      return "bg-orange-900/40 text-orange-300";
    case "expression":
      return "bg-pink-900/40 text-pink-300";
    case "technique":
      return "bg-teal-900/40 text-teal-300";
    default:
      return "bg-gray-700/40 text-gray-300";
  }
}

/**
 * A single coaching tip with auto-dismiss timer. Slides in from the
 * right and fades out after ~10 seconds, or on explicit dismiss.
 */
function TipCard({
  id,
  text,
  severity,
  category,
  onDismiss,
}: {
  id: string;
  text: string;
  severity: CoachingSeverity;
  category: CoachingCategory;
  onDismiss: (id: string) => void;
}) {
  const [isDismissing, setIsDismissing] = useState(false);

  useEffect(() => {
    const timer = setTimeout(() => {
      setIsDismissing(true);
      setTimeout(() => onDismiss(id), 300); // Wait for fade-out animation
    }, 10000);

    return () => clearTimeout(timer);
  }, [id, onDismiss]);

  const handleDismiss = () => {
    setIsDismissing(true);
    setTimeout(() => onDismiss(id), 300);
  };

  return (
    <div
      data-testid={`coaching-tip-${id}`}
      className={`
        transform transition-all duration-300 ease-out
        ${isDismissing ? "translate-x-full opacity-0" : "translate-x-0 opacity-100"}
      `}
    >
      <div
        className={`
          rounded-lg border p-4 shadow-lg
          ${severityColor(severity)}
        `}
      >
        <div className="flex items-start justify-between gap-3">
          <div className="flex-1">
            <p className="text-sm font-medium leading-relaxed">{text}</p>
            <div className="mt-2 flex gap-2">
              <span
                className={`
                  rounded px-2 py-1 text-xs font-semibold uppercase tracking-wider
                  ${severityColor(severity)}
                `}
              >
                {severity}
              </span>
              <span
                className={`
                  rounded px-2 py-1 text-xs font-semibold uppercase tracking-wider
                  ${categoryColor(category)}
                `}
              >
                {category}
              </span>
            </div>
          </div>

          <button
            type="button"
            onClick={handleDismiss}
            data-testid={`tip-dismiss-${id}`}
            className="mt-0.5 inline-flex h-6 w-6 items-center justify-center rounded-full text-sm font-bold opacity-60 hover:opacity-100"
            aria-label={`Dismiss coaching tip: ${text}`}
          >
            ×
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * Coaching tip panel positioned alongside the pitch display. Displays
 * queued tips as a vertical stack, auto-dismissing each after ~10
 * seconds. Tips slide in from the right with a subtle fade, maintaining
 * the "calm, encouraging" tone of the session.
 */
export default function CoachingTipPanel() {
  const tipQueue = usePracticeStore((s) => s.tipQueue);
  const dismissTip = usePracticeStore((s) => s.dismissTip);

  // Show only the most recent tip (don't stack multiple tips)
  const currentTip = tipQueue.length > 0 ? tipQueue[tipQueue.length - 1] : null;

  if (!currentTip) {
    return (
      <div
        className="h-32 w-64"
        data-testid="coaching-tip-panel-empty"
        aria-label="No coaching tips at this time"
      />
    );
  }

  return (
    <div
      className="w-64"
      data-testid="coaching-tip-panel"
      role="status"
      aria-live="polite"
      aria-label="Coaching tip panel"
    >
      <TipCard
        id={currentTip.id}
        text={currentTip.tip.text}
        severity={currentTip.tip.severity}
        category={currentTip.tip.category}
        onDismiss={dismissTip}
      />
    </div>
  );
}
