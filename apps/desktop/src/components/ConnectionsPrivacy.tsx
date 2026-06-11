import { useConnectionsStore } from "../stores/connectionsStore";
import { usePracticeStore } from "../stores/practiceStore";

/**
 * Connections & Privacy — the one place that names every feature which can
 * touch the network, in plain language, with an opt-in toggle that starts off.
 *
 * This is the Face-layer expression of the product principle in
 * `docs/architecture/offline-first-and-network-transparency.md`:
 *
 *   Offline by default. The internet is NEVER required for core value.
 *   Every networked feature is opt-in, off by default, and discloses what
 *   leaves the device.
 *
 * Tone is "coach, don't judge": we explain the trade honestly and let the
 * user choose. No dark patterns, no pre-checked boxes, no guilt.
 */

interface ToggleRowProps {
  id: string;
  title: string;
  /** Plain-language description of what is sent and to whom. */
  description: string;
  /** What happens when this is off — the on-device behaviour. */
  whenOff: string;
  enabled: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
}

function ToggleRow({
  id,
  title,
  description,
  whenOff,
  enabled,
  onChange,
  disabled = false,
}: ToggleRowProps) {
  return (
    <div className="rounded-lg bg-gray-800 p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h3 className="font-medium text-white">{title}</h3>
          <p className="mt-1 text-sm text-gray-300">{description}</p>
          <p className="mt-2 text-sm text-gray-400">
            <span className="font-medium text-gray-300">When off:</span>{" "}
            {whenOff}
          </p>
        </div>
        <label
          htmlFor={id}
          className="flex shrink-0 cursor-pointer items-center gap-2 text-sm text-gray-300"
        >
          <span>{enabled ? "On" : "Off"}</span>
          <input
            id={id}
            type="checkbox"
            role="switch"
            aria-label={title}
            checked={enabled}
            disabled={disabled}
            onChange={(e) => onChange(e.target.checked)}
            className="h-5 w-5 rounded border-gray-600 bg-gray-900 accent-indigo-500 disabled:opacity-40"
          />
        </label>
      </div>
    </div>
  );
}

export default function ConnectionsPrivacy() {
  const goToHistory = usePracticeStore((s) => s.goToHistory);

  // AI coaching narration reuses its long-standing preference.
  const coachingEnabled = usePracticeStore((s) => s.coachingEnabled);
  const setCoachingEnabled = usePracticeStore((s) => s.setCoachingEnabled);

  // Cloud sync + teacher sharing opt-in intent (off by default).
  const cloudSyncEnabled = useConnectionsStore((s) => s.cloudSyncEnabled);
  const setCloudSyncEnabled = useConnectionsStore((s) => s.setCloudSyncEnabled);
  const teacherSharingEnabled = useConnectionsStore(
    (s) => s.teacherSharingEnabled,
  );
  const setTeacherSharingEnabled = useConnectionsStore(
    (s) => s.setTeacherSharingEnabled,
  );

  return (
    <main
      className="min-h-screen bg-gray-900 p-8 text-white"
      data-testid="connections-privacy-panel"
    >
      <div className="mx-auto max-w-2xl">
        <h1 className="text-4xl font-bold">Connections &amp; Privacy</h1>
        <p className="mt-3 text-gray-300">
          You&rsquo;re in control of everything that uses the internet. Each
          feature below is off until you turn it on, and we tell you exactly
          what leaves your device.
        </p>

        {/* Standing reassurance — always present, never conditional. */}
        <p
          className="mt-4 rounded-lg border border-emerald-700/50 bg-emerald-900/20 p-4 text-sm text-emerald-200"
          data-testid="offline-reassurance"
        >
          Everything else works offline. Practice, real-time feedback, and your
          session recap never need the internet &mdash; they all run right here
          on your device.
        </p>

        <div className="mt-8 space-y-4">
          <ToggleRow
            id="toggle-ai-coaching"
            title="AI coaching narration"
            description="Lets the coach phrase its feedback with an AI writer. It sends your performance numbers — instrument, durations, pitch, tone, intonation and timing figures — to the AI provider. It never sends your audio recording."
            whenOff="Your coach still works, using fully on-device feedback. Tips and recaps read a little more general, but every word is based only on what was measured here, with nothing sent anywhere."
            enabled={coachingEnabled}
            onChange={setCoachingEnabled}
          />

          <ToggleRow
            id="toggle-cloud-sync"
            title="Cloud sync"
            description="Backs up your finished session recaps so you can see them on another device. It sends the recap summary — instrument, dates, duration, and the written notes — to our secure cloud. It never sends your audio recording. (Syncing also requires signing in.)"
            whenOff="Your full practice history stays on this device only. Nothing is uploaded."
            enabled={cloudSyncEnabled}
            onChange={setCloudSyncEnabled}
          />

          <ToggleRow
            id="toggle-teacher-sharing"
            title="Share with a teacher"
            description="Lets a teacher you link see your synced session recaps. It shares the same recap summaries cloud sync already uploads — no audio. Works only when cloud sync is on."
            whenOff="No teacher can see your sessions. Sharing is entirely your choice."
            enabled={teacherSharingEnabled && cloudSyncEnabled}
            disabled={!cloudSyncEnabled}
            onChange={setTeacherSharingEnabled}
          />
        </div>

        <p className="mt-6 text-sm text-gray-500">
          We never turn these on for you, never run them in the background, and
          never require an account to practice. No usage tracking ships on by
          default.
        </p>

        <button
          onClick={() => goToHistory()}
          className="mt-8 rounded-md border border-gray-600 px-4 py-2 text-sm text-gray-300 hover:bg-gray-700"
        >
          Back
        </button>
      </div>
    </main>
  );
}
