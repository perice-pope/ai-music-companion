import { useConnectionsStore } from "../stores/connectionsStore";
import { usePracticeStore } from "../stores/practiceStore";
import AppVersionBadge from "./AppVersionBadge";

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

interface InfoRowProps {
  testId: string;
  title: string;
  /** Plain-language description of what is sent, to whom, and when. */
  description: string;
  /** What the default behaviour is — i.e. no network unless you ask. */
  whenIdle: string;
}

/**
 * A disclosure row with NO toggle. Used for networked behaviour the Face layer
 * does not itself gate — here, the Tauri app auto-updater, whose check/download
 * is user-initiated through the native update dialog (never on startup) and
 * lives below the JS boundary in the `tauri-plugin-updater` dependency. We
 * disclose it honestly rather than render a switch that wouldn't actually
 * control the egress (that would be a dark pattern). It carries a "status"
 * pill instead of a control, so it is not counted among the off-by-default
 * toggles.
 */
function InfoRow({ testId, title, description, whenIdle }: InfoRowProps) {
  return (
    <div className="rounded-lg bg-gray-800 p-4" data-testid={testId}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h3 className="font-medium text-white">{title}</h3>
          <p className="mt-1 text-sm text-gray-300">{description}</p>
          <p className="mt-2 text-sm text-gray-400">
            <span className="font-medium text-gray-300">By default:</span>{" "}
            {whenIdle}
          </p>
        </div>
        <span className="shrink-0 rounded-full border border-gray-600 px-3 py-1 text-xs text-gray-400">
          Only when you ask
        </span>
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
  const autoUpdateCheckEnabled = useConnectionsStore(
    (s) => s.autoUpdateCheckEnabled,
  );
  const setAutoUpdateCheckEnabled = useConnectionsStore(
    (s) => s.setAutoUpdateCheckEnabled,
  );
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
            description="Lets the coach phrase its feedback with an AI writer. It sends your performance numbers — instrument, durations, pitch, tone, intonation and timing figures — to the AI provider. It also lets the ‘in the wild’ music reveals reword their one-line explanation (sending only the detected key/scale and the fixed artist/piece, never a new one). It never sends your audio recording."
            whenOff="Your coach still works, using fully on-device feedback. Tips, recaps, and the music reveals read a little more general, but every word is based only on what was measured here, with nothing sent anywhere."
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

          <ToggleRow
            id="toggle-auto-update-check"
            title="Check for updates automatically"
            description="Lets the app quietly ask GitHub for the latest version when it opens (and every few hours while running). If a newer signed build exists, a small pill appears bottom-left — nothing downloads until you click it. The check sends no audio, no practice history, no personal data — just a request for the latest version number."
            whenOff="The app makes no update request on launch or in the background. A check only happens when you choose \u201cCheck for updates\u201d yourself, and the app works fully offline forever."
            enabled={autoUpdateCheckEnabled}
            onChange={setAutoUpdateCheckEnabled}
          />

          <InfoRow
            testId="info-app-updates"
            title="App updates"
            description="Checking for updates contacts GitHub only when you ask (or automatically, if you turned that on above). If an update is found, it asks you before downloading the new signed version. The app never checks on startup and works fully offline. It never sends your audio, your practice history, or any personal data — just a request for the latest version."
            whenIdle={
              "Nothing is sent. The app makes no update request on launch or in the background; a check only happens when you choose “Check for updates.” You can keep using the app without ever checking."
            }
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

        {/* Which build am I running? Quotable in bug reports (#384). */}
        <AppVersionBadge className="mt-6" />
      </div>
    </main>
  );
}
