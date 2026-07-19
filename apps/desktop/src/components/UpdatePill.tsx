import { useState } from "react";
import { useUpdateStore } from "../stores/updateStore";

/**
 * #58 — the bottom-left update pill, now with the window (E1).
 *
 * ONE persistent element across every phase and expansion state
 * (#417 rule 0): clicking an available update GROWS the pill in place
 * into a compact card — "v{X} — what's new", the release notes, and
 * "Update now" / "Not now" — then dims through downloading and offers a
 * real "Restart now" once staged. Nothing flashes, nothing remounts.
 *
 * It only ever appears when a check found a newer signed build — and
 * checks only happen when the user opted in (or asked).
 */
export default function UpdatePill() {
  const phase = useUpdateStore((s) => s.phase);
  const version = useUpdateStore((s) => s.availableVersion);
  const notice = useUpdateStore((s) => s.notice);
  const releaseNotes = useUpdateStore((s) => s.releaseNotes);
  const restartFailed = useUpdateStore((s) => s.restartFailed);
  const installUpdate = useUpdateStore((s) => s.installUpdate);
  const restartNow = useUpdateStore((s) => s.restartNow);
  const dismiss = useUpdateStore((s) => s.dismiss);
  // Review MF1: expansion is pinned to the OFFER — a dismissed window's
  // state must never leak so a later version pops open uninvited.
  const [expandedFor, setExpandedFor] = useState<string | null>(null);

  if (phase === "idle") {
    return null;
  }

  const expanded = expandedFor === version;
  const dimmed = phase === "downloading" || phase === "error";

  return (
    <div
      data-testid="update-pill"
      data-phase={phase}
      data-expanded={expanded}
      className={`fixed bottom-4 left-4 z-40 rounded-2xl
        border border-sky-700 bg-sky-950/95 text-sm text-sky-100
        shadow-lg transition-all duration-300
        ${dimmed ? "opacity-60" : "opacity-100"}
        ${expanded && phase === "available" ? "w-80 p-4" : "px-2 py-1.5"}`}
    >
      {phase === "available" && !expanded && (
        <div className="flex items-center gap-2 pl-2">
          <button
            type="button"
            data-testid="update-pill-install"
            onClick={() => setExpandedFor(version)}
            className="font-semibold hover:text-white"
          >
            Update to v{version}
          </button>
          <button
            type="button"
            data-testid="update-pill-dismiss"
            onClick={() => {
              setExpandedFor(null);
              dismiss();
            }}
            aria-label="Dismiss update"
            className="inline-flex h-6 w-6 items-center justify-center rounded-full text-sky-300/70 hover:text-white"
          >
            ×
          </button>
        </div>
      )}

      {phase === "available" && expanded && (
        <div data-testid="update-window">
          <div className="flex items-start justify-between gap-2">
            <p className="font-semibold" data-testid="update-window-title">
              v{version} — what's new
            </p>
            <button
              type="button"
              data-testid="update-pill-dismiss"
              onClick={() => {
                setExpandedFor(null);
                dismiss();
              }}
              aria-label="Dismiss update"
              className="inline-flex h-6 w-6 items-center justify-center rounded-full text-sky-300/70 hover:text-white"
            >
              ×
            </button>
          </div>
          <div
            data-testid="update-window-notes"
            className="mt-2 max-h-40 overflow-y-auto whitespace-pre-wrap text-sky-200/90"
          >
            {releaseNotes?.trim() || "No notes for this release."}
          </div>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              data-testid="update-window-install"
              onClick={() => void installUpdate()}
              className="rounded-md bg-sky-600 px-3 py-1.5 font-semibold text-white hover:bg-sky-500"
            >
              Update now
            </button>
            <button
              type="button"
              data-testid="update-window-later"
              onClick={() => setExpandedFor(null)}
              className="rounded-md border border-sky-800 px-3 py-1.5 text-sky-200 hover:bg-sky-900/40"
            >
              Not now
            </button>
          </div>
        </div>
      )}

      {phase === "downloading" && (
        <span className="pl-2" data-testid="update-pill-label">
          Updating…
        </span>
      )}

      {phase === "ready" &&
        (restartFailed ? (
          <span className="pl-2" data-testid="update-pill-label">
            Quit and reopen to finish the update
          </span>
        ) : (
          <button
            type="button"
            data-testid="update-pill-restart"
            onClick={() => void restartNow()}
            className="pl-2 font-semibold hover:text-white"
          >
            Restart now
          </button>
        ))}

      {phase === "error" && (
        <div className="flex items-center gap-2 pl-2">
          <span data-testid="update-pill-label">
            {notice ?? "The update didn't finish."}
          </span>
          <button
            type="button"
            data-testid="update-pill-dismiss"
            onClick={() => {
              setExpandedFor(null);
              dismiss();
            }}
            aria-label="Dismiss update"
            className="inline-flex h-6 w-6 items-center justify-center rounded-full text-sky-300/70 hover:text-white"
          >
            ×
          </button>
        </div>
      )}
    </div>
  );
}
