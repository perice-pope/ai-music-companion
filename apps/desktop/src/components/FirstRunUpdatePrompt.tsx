import { useConnectionsStore } from "../stores/connectionsStore";
import { useUpdateStore } from "../stores/updateStore";

/**
 * #465 — the once-only first-launch question, in the update pill's spot.
 *
 * The #58 auto-check is strictly opt-in, but nothing ever told a new user
 * the toggle exists — a fresh install sits silently on a stale version and
 * the pill's first impression is absence. This asks the question once, in
 * plain words; either answer is final here, and changing your mind lives in
 * Connections & Privacy, which the copy names.
 *
 * Offline-first: the prompt itself makes no network call. "Yes" flips the
 * same store switch the settings row flips; the App heartbeat reacts to
 * that flag, so no code path here can check before the answer. It yields
 * the slot whenever the pill has something to say (any non-idle phase), so
 * the two bottom-left elements never stack.
 */
export default function FirstRunUpdatePrompt() {
  const answered = useConnectionsStore((s) => s.updatePromptAnswered);
  const autoUpdateOn = useConnectionsStore((s) => s.autoUpdateCheckEnabled);
  const answerUpdatePrompt = useConnectionsStore((s) => s.answerUpdatePrompt);
  const pillPhase = useUpdateStore((s) => s.phase);

  if (answered || autoUpdateOn || pillPhase !== "idle") {
    return null;
  }

  return (
    <div
      data-testid="first-run-update-prompt"
      className="fixed bottom-4 left-4 z-40 w-80 rounded-2xl border border-sky-700
        bg-sky-950/95 p-4 text-sm text-sky-100 shadow-lg"
    >
      <p>
        Check for updates automatically? One request to GitHub for a version
        file &mdash; that&rsquo;s the only thing it ever does. You can change
        this anytime in Connections &amp; Privacy.
      </p>
      <div className="mt-3 flex gap-2">
        <button
          type="button"
          data-testid="update-prompt-yes"
          onClick={() => answerUpdatePrompt(true)}
          className="rounded-md bg-sky-600 px-3 py-1.5 font-semibold text-white hover:bg-sky-500"
        >
          Yes, keep me current
        </button>
        <button
          type="button"
          data-testid="update-prompt-no"
          onClick={() => answerUpdatePrompt(false)}
          className="rounded-md border border-sky-800 px-3 py-1.5 text-sky-200 hover:bg-sky-900/40"
        >
          No thanks
        </button>
      </div>
    </div>
  );
}
