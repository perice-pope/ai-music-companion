import { useUpdateStore } from "../stores/updateStore";

/**
 * #58 — the bottom-left update pill, Claude-desktop style.
 *
 * ONE persistent element across every phase (#417 rule 0): "Update to
 * vX.Y.Z" → dimmed "Updating…" → "Quit and reopen to finish" → a calm
 * error line. Phases replace text and opacity in place; the pill never
 * flashes, and it renders nothing at all while idle (no empty chrome).
 *
 * It only ever appears when a check has found a newer signed build — and
 * checks only happen when the user opted in (see App + connectionsStore).
 */
export default function UpdatePill() {
  const phase = useUpdateStore((s) => s.phase);
  const version = useUpdateStore((s) => s.availableVersion);
  const notice = useUpdateStore((s) => s.notice);
  const installUpdate = useUpdateStore((s) => s.installUpdate);
  const dismiss = useUpdateStore((s) => s.dismiss);

  if (phase === "idle") {
    return null;
  }

  const dimmed = phase === "downloading" || phase === "error";
  const label =
    phase === "available"
      ? `Update to v${version}`
      : phase === "downloading"
        ? "Updating…"
        : phase === "ready"
          ? "Quit and reopen to finish the update"
          : (notice ?? "The update didn't finish.");

  return (
    <div
      data-testid="update-pill"
      data-phase={phase}
      className={`fixed bottom-4 left-4 z-40 flex items-center gap-2 rounded-full
        border border-sky-700 bg-sky-950/90 py-1.5 pl-4 pr-2 text-sm text-sky-100
        shadow-lg transition-opacity duration-700
        ${dimmed ? "opacity-60" : "opacity-100"}`}
    >
      {phase === "available" ? (
        <button
          type="button"
          data-testid="update-pill-install"
          onClick={() => void installUpdate()}
          className="font-semibold hover:text-white"
        >
          {label}
        </button>
      ) : (
        <span data-testid="update-pill-label">{label}</span>
      )}
      {(phase === "available" || phase === "error") && (
        <button
          type="button"
          data-testid="update-pill-dismiss"
          onClick={dismiss}
          aria-label="Dismiss update"
          className="inline-flex h-6 w-6 items-center justify-center rounded-full text-sky-300/70 hover:text-white"
        >
          ×
        </button>
      )}
    </div>
  );
}
