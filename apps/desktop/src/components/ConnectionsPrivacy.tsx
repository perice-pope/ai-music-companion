import { useState } from "react";
import { useConnectionsStore } from "../stores/connectionsStore";
import { useUpdateStore } from "../stores/updateStore";
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
/**
 * #58 review MF4: the manual "Check for updates" the disclosure copy has
 * always described — now it actually exists. User-initiated, independent
 * of the automatic-check toggle; a found update surfaces the pill, an
 * up-to-date answer says so calmly right here.
 */
function CheckForUpdatesButton() {
  const checkForUpdate = useUpdateStore((s) => s.checkForUpdate);
  const [state, setState] = useState<"idle" | "checking" | "done">("idle");
  const [result, setResult] = useState<string | null>(null);

  const onCheck = async () => {
    setState("checking");
    // manual: an explicit ask overrides a pill dismissal (round-2 MF1) —
    // and the answer comes from what the check LEARNED, never inferred
    // from phase (which lied when offline or dismissed).
    const outcome = await checkForUpdate({ manual: true });
    setResult(
      outcome === "upToDate"
        ? "You're on the latest version."
        : outcome === "found"
          ? "Update found — see the pill, bottom left."
          : outcome === "busy"
            ? "An update is already in progress — see the pill."
            : "Couldn't check right now — you may be offline. Nothing was sent.",
    );
    setState("done");
  };

  return (
    <div className="mt-2 flex items-center gap-3">
      <button
        type="button"
        data-testid="check-updates-now"
        onClick={() => void onCheck()}
        disabled={state === "checking"}
        className="rounded-md border border-gray-600 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-700 disabled:opacity-50"
      >
        {state === "checking" ? "Checking…" : "Check for updates"}
      </button>
      {state === "done" && result && (
        <span
          className="text-sm text-gray-400"
          data-testid="check-updates-result"
        >
          {result}
        </span>
      )}
    </div>
  );
}

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
  // #449 T2: the dashboard projection's own opt-in (rides on cloud sync).
  const dashboardSyncEnabled = useConnectionsStore(
    (s) => s.dashboardSyncEnabled,
  );
  const setDashboardSyncEnabled = useConnectionsStore(
    (s) => s.setDashboardSyncEnabled,
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
            description="Lets the coach phrase its feedback with an AI writer. It sends your performance numbers — instrument, durations, pitch, tone, intonation and timing figures — to the AI provider. It also lets the ‘in the wild’ music reveals reword their one-line explanation (sending only the detected scale/mode sound and the fixed artist/piece, never a new one). It never sends your audio recording."
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
            id="toggle-dashboard-sync"
            title="Share practice details with your teacher's dashboard"
            description="Gives a teacher whose classroom you've joined a fuller picture of how you practice. On top of the recaps cloud sync uploads, it sends: session facts (when you practiced, minutes of actual playing vs. minutes the app was open, how many notes were heard), the start and end times of each phrase you played, the names and scores of exercises you worked on (like ‘Minor triad, enclosed, descending’ — the name and your accuracy, never the exercise recipe itself), the title of any piece you practiced, and which practice tools you used (metronome on/off and tempo, backing band, opener, when you opened a piece — its id — and whether the AI coach's narration was used). Only a teacher you joined through a classroom code can see it. It never sends your audio recording, the actual notes you played, or your saved exercise recipes and seeds. Works only when cloud sync is on."
            whenOff="Your teacher sees only what cloud sync and teacher sharing already allow — session recaps at most. None of the practice details above leave this device."
            enabled={dashboardSyncEnabled && cloudSyncEnabled}
            disabled={!cloudSyncEnabled}
            onChange={setDashboardSyncEnabled}
          />

          <ToggleRow
            id="toggle-auto-update-check"
            title="Check for updates automatically"
            description="Lets the app quietly ask GitHub for the latest version when it opens (and every few hours while running). If a newer signed build exists, a small pill appears bottom-left — nothing downloads until you click it. The check sends no audio, no practice history, no personal data — just a request for the latest version number."
            whenOff="The app makes no update request on launch or in the background. A check only happens when you press the check button below yourself, and the app works fully offline forever."
            enabled={autoUpdateCheckEnabled}
            onChange={setAutoUpdateCheckEnabled}
          />

          {/* #449 enrollment slice: join/leave calls are user-initiated
              (explicit buttons + the in-app consent screen), so this is a
              disclosure row, not a toggle — a switch here wouldn't gate
              anything real (dark pattern). What a teacher can SEE is governed
              by the sync toggles above. */}
          <InfoRow
            testId="info-classroom-enrollment"
            title="Classroom enrollment"
            description="Joining a teacher's classroom sends the join code you type, your consent choice (you, or a parent/guardian for under-13), and your age group (never a birthdate) to the app's cloud service, which records the enrollment. Leaving a classroom sends the revoke, which cuts off your teacher's view immediately. For teachers: creating a classroom sends its name, and issuing a join code asks the server to mint one. Every call requires signing in and happens only when you press the button — enrollment itself shares no practice data; what a teacher can see is controlled by the sync switches above."
            whenIdle="Nothing is sent. Enrollment calls happen only when you ask — joining, leaving, or issuing a code — never in the background."
          />

          <InfoRow
            testId="info-app-updates"
            title="App updates"
            description="Checking for updates contacts GitHub only when you ask — or automatically, if you turned that on above. If an update is found, the pill asks you before downloading the new signed version. Unless automatic checks are on, the app never checks on startup, and it works fully offline either way. It never sends your audio, your practice history, or any personal data — just a request for the latest version."
            whenIdle={
              "Nothing is sent. The app makes no update request on launch or in the background; a check only happens when you choose “Check for updates.” You can keep using the app without ever checking."
            }
          />
          <CheckForUpdatesButton />
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
