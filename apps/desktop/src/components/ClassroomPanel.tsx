import { useEffect, useState } from "react";
import { useAuthStore } from "../stores/authStore";
import { useConnectionsStore } from "../stores/connectionsStore";
import {
  useEnrollmentStore,
  type AgeTier,
  type EnrollmentRow,
} from "../stores/enrollmentStore";

/**
 * #449 enrollment slice — classroom enrollment on the History page
 * (spec: docs/specs/449-enrollment.md).
 *
 * Student side: "Join a classroom" (code → consent screen → redeem_join_code
 * → dashboard-sync prompt), current enrollments with a self-serve Leave.
 * Teacher side (profiles.role='teacher'): "My classroom" — create, issue/show
 * the join code (expiry read from the row, i.e. the server's clock), roster
 * count.
 *
 * Consent is the gate: the redeem RPC fires ONLY from the explicit accept on
 * the consent screen, which names in plain words everything a teacher can see
 * (the T2 disclosure list). Under-13 (profiles.age_tier): a parent/guardian
 * must complete the step — the student cannot self-consent — and the redeem
 * goes up as consent='parent' (COPPA gate; the server enforces the same rule).
 *
 * Signed out this renders nothing: enrollment is meaningless without an
 * account, and AccountPanel directly above already offers sign-in.
 */

type FlowStep = "idle" | "code" | "age" | "consent" | "syncPrompt";

/**
 * The consent screen's disclosure list — the five sync surfaces of
 * docs/architecture/teacher-dashboard-datamodel.md §2 (P1–P5), in plain
 * words. Exported for the tests that pin the copy.
 */
export const CONSENT_DISCLOSURE_ITEMS: ReadonlyArray<{
  title: string;
  body: string;
}> = [
  {
    title: "Session recaps",
    body: "your instrument, practice dates, session length, and the written recap notes.",
  },
  {
    title: "Session facts",
    body: "when you practiced, minutes of actual playing vs. minutes the app was open, and how many notes were heard.",
  },
  {
    title: "Phrase timings",
    body: "when each phrase you played started and ended, with its note count, steadiness and tone figures, and the name of the key we heard.",
  },
  {
    title: "Exercises",
    body: "the names of exercises you worked on (like ‘Minor triad, enclosed, descending’), their difficulty, and your accuracy — never the exercise recipe or seed.",
  },
  {
    title: "Tools and progress",
    body: "which practice tools you used (metronome and tempo, backing band, opener, opening a piece and its title, whether AI narration was used) and your per-key progress stats.",
  },
];

export const CONSENT_NEVER_LINE =
  "Never shared, with anyone: your audio recordings, the actual notes you played, or your saved exercise recipes and seeds.";

export const CONSENT_SHARING_NOTE =
  "Joining alone shares nothing. Your teacher sees practice data only if you also turn on cloud sync and dashboard sharing — we’ll ask right after this. You can leave the classroom at any time, and your teacher’s view closes immediately.";

export const UNDER_13_NOTICE =
  "Because this student is under 13, a parent or legal guardian must complete this step — a student under 13 can’t agree to this on their own.";

export const GUARDIAN_ACK_LABEL =
  "I am this student’s parent or legal guardian, and I agree to what’s described above.";

function formatDate(iso: string | null): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

function EnrollmentItem({
  enrollment,
  working,
  onLeave,
}: {
  enrollment: EnrollmentRow;
  working: boolean;
  onLeave: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const joined = formatDate(enrollment.joined_via_code_at);
  return (
    <div
      className="flex items-center justify-between gap-4 rounded-md border border-gray-700 p-3"
      data-testid={`enrollment-${enrollment.id}`}
    >
      <div className="min-w-0 text-sm">
        {/* Students cannot read classroom names (RLS) — say what we know,
            never invent (silence > lies). */}
        <p className="text-white">
          {enrollment.status === "active"
            ? "Enrolled in a classroom"
            : enrollment.status === "invited"
              ? "Invited to a classroom — enter its join code to accept"
              : "Left a classroom"}
        </p>
        <p className="mt-0.5 text-gray-400">
          {joined ? `Joined ${joined}. ` : ""}
          {enrollment.status === "active" &&
            (enrollment.consent === "parent"
              ? "A parent or guardian agreed to this."
              : "You agreed to this.")}
        </p>
      </div>
      {enrollment.status === "active" &&
        (confirming ? (
          <span className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              data-testid={`confirm-leave-${enrollment.id}`}
              disabled={working}
              onClick={onLeave}
              className="rounded-md border border-red-700 px-3 py-1.5 text-sm text-red-300 hover:bg-red-900/30 disabled:opacity-50"
            >
              Yes, leave
            </button>
            <button
              type="button"
              onClick={() => setConfirming(false)}
              className="text-sm text-gray-400 hover:text-gray-200"
            >
              Keep
            </button>
          </span>
        ) : (
          <button
            type="button"
            data-testid={`leave-${enrollment.id}`}
            onClick={() => setConfirming(true)}
            className="shrink-0 rounded-md border border-gray-600 px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700"
          >
            Leave
          </button>
        ))}
    </div>
  );
}

export default function ClassroomPanel() {
  const user = useAuthStore((s) => s.user);
  const status = useAuthStore((s) => s.status);
  const store = useEnrollmentStore();
  const setCloudSyncEnabled = useConnectionsStore((s) => s.setCloudSyncEnabled);
  const setDashboardSyncEnabled = useConnectionsStore(
    (s) => s.setDashboardSyncEnabled,
  );

  const [step, setStep] = useState<FlowStep>("idle");
  const [code, setCode] = useState("");
  const [ageChoice, setAgeChoice] = useState<AgeTier | null>(null);
  const [guardianAck, setGuardianAck] = useState(false);
  const [classroomName, setClassroomName] = useState("");

  const userId = user?.id;
  const loadProfile = store.loadProfile;
  const loadEnrollments = store.loadEnrollments;
  const loadClassrooms = store.loadClassrooms;
  const isTeacher = store.profile?.role === "teacher";

  useEffect(() => {
    void loadProfile(userId);
    void loadEnrollments(userId);
  }, [userId, loadProfile, loadEnrollments]);

  useEffect(() => {
    if (isTeacher) void loadClassrooms(userId);
  }, [isTeacher, userId, loadClassrooms]);

  // Signed out (or still restoring): nothing to enroll with, nothing shown.
  if (status !== "signed_in" || !userId) return null;

  const ageTier = store.profile?.ageTier ?? null;
  const underThirteen = ageTier === "under_13";

  const resetFlow = () => {
    setStep("idle");
    setCode("");
    setAgeChoice(null);
    setGuardianAck(false);
    store.clearError();
  };

  const continueFromCode = () => {
    if (!code.trim()) return;
    store.clearError();
    setStep(ageTier === null ? "age" : "consent");
  };

  const continueFromAge = async () => {
    if (!ageChoice) return;
    const ok = await store.setAgeTier(userId, ageChoice);
    // The DB-side under-13 gate must bind against a real tier before consent
    // is even offered; a failed write keeps us here, calmly.
    if (ok) setStep("consent");
  };

  const accept = async (consent: "student" | "parent") => {
    const ok = await store.redeemJoinCode(userId, code, consent);
    if (ok) setStep("syncPrompt");
  };

  const enableSharing = () => {
    // The dependency rule: dashboard sharing rides on cloud sync — the prompt
    // enables both, and says so. Both remain user-reversible in
    // Connections & Privacy.
    setCloudSyncEnabled(true);
    setDashboardSyncEnabled(true);
    resetFlow();
  };

  const visibleEnrollments = store.enrollments.filter(
    (e) => e.status === "active" || e.status === "invited",
  );

  return (
    <div className="mt-3 space-y-3" data-testid="classroom-panel">
      {/* ── Student side: enrollments + the join flow ── */}
      <div className="rounded-lg bg-gray-800 p-4">
        <div className="flex items-center justify-between gap-4">
          <h3 className="font-medium text-white">Classroom</h3>
          {step === "idle" && (
            <button
              type="button"
              data-testid="join-classroom-open"
              onClick={() => setStep("code")}
              className="rounded-md border border-gray-600 px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700"
            >
              Join a classroom
            </button>
          )}
        </div>

        {visibleEnrollments.length > 0 ? (
          <div className="mt-3 space-y-2">
            {visibleEnrollments.map((e) => (
              <EnrollmentItem
                key={e.id}
                enrollment={e}
                working={store.working}
                onLeave={() => void store.leaveClassroom(userId, e.id)}
              />
            ))}
          </div>
        ) : (
          step === "idle" && (
            <p className="mt-2 text-sm text-gray-400">
              Not in a classroom. If your teacher gave you a join code, you can
              enroll here — nothing is shared without your say-so.
            </p>
          )
        )}

        {store.error && step === "idle" && (
          <p className="mt-3 text-sm text-red-300" role="alert">
            {store.error}
          </p>
        )}

        {/* Step: code entry */}
        {step === "code" && (
          <div className="mt-3" data-testid="join-code-step">
            <label
              htmlFor="join-code-input"
              className="block text-sm text-gray-300"
            >
              Enter the join code from your teacher
            </label>
            <div className="mt-2 flex items-center gap-2">
              <input
                id="join-code-input"
                type="text"
                aria-label="Join code"
                placeholder="e.g. ABCD2345"
                value={code}
                onChange={(e) => setCode(e.target.value.toUpperCase())}
                className="w-40 rounded-md border border-gray-600 bg-gray-900 px-3 py-2 text-sm tracking-widest text-white placeholder-gray-500"
              />
              <button
                type="button"
                data-testid="join-code-continue"
                disabled={!code.trim()}
                onClick={continueFromCode}
                className="rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
              >
                Continue
              </button>
              <button
                type="button"
                onClick={resetFlow}
                className="text-sm text-gray-400 hover:text-gray-200"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Step: age group (only when the profile doesn't carry one yet) */}
        {step === "age" && (
          <div className="mt-3" data-testid="age-step">
            <p className="text-sm font-medium text-white">
              One quick question first
            </p>
            <p className="mt-1 text-sm text-gray-300">
              How old is the musician using this account? We store only the age
              group — never a birthdate. Under-13 accounts need a parent or
              guardian to approve classroom sharing.
            </p>
            <div className="mt-2 space-y-1">
              {(
                [
                  ["under_13", "Under 13"],
                  ["teen_13_17", "13–17"],
                  ["adult", "18 or older"],
                ] as const
              ).map(([tier, label]) => (
                <label
                  key={tier}
                  className="flex items-center gap-2 text-sm text-gray-200"
                >
                  <input
                    type="radio"
                    name="age-tier"
                    value={tier}
                    checked={ageChoice === tier}
                    onChange={() => setAgeChoice(tier)}
                    className="accent-indigo-500"
                  />
                  {label}
                </label>
              ))}
            </div>
            {store.error && (
              <p className="mt-2 text-sm text-red-300" role="alert">
                {store.error}
              </p>
            )}
            <div className="mt-3 flex items-center gap-3">
              <button
                type="button"
                data-testid="age-continue"
                disabled={!ageChoice || store.working}
                onClick={() => void continueFromAge()}
                className="rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
              >
                Continue
              </button>
              <button
                type="button"
                onClick={resetFlow}
                className="text-sm text-gray-400 hover:text-gray-200"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Step: THE consent screen — the only path to the redeem RPC. */}
        {step === "consent" && (
          <div className="mt-3" data-testid="consent-screen">
            <p className="font-medium text-white">
              Before you join: what your teacher can see
            </p>
            <p className="mt-1 text-sm text-gray-300">
              Joining links your account to your teacher&rsquo;s roster. Here is
              the complete list of what your teacher can see — nothing outside
              this list ever leaves your device:
            </p>
            <ul className="mt-2 list-disc space-y-1 pl-5 text-sm text-gray-300">
              {CONSENT_DISCLOSURE_ITEMS.map((item) => (
                <li key={item.title}>
                  <span className="font-medium text-gray-200">
                    {item.title}
                  </span>{" "}
                  &mdash; {item.body}
                </li>
              ))}
            </ul>
            <p className="mt-2 rounded-md border border-emerald-700/50 bg-emerald-900/20 p-2 text-sm text-emerald-200">
              {CONSENT_NEVER_LINE}
            </p>
            <p className="mt-2 text-sm text-gray-400">{CONSENT_SHARING_NOTE}</p>

            {underThirteen && (
              <div className="mt-3" data-testid="consent-under-13">
                <p className="text-sm font-medium text-amber-200">
                  {UNDER_13_NOTICE}
                </p>
                <label className="mt-2 flex items-start gap-2 text-sm text-gray-200">
                  <input
                    type="checkbox"
                    data-testid="guardian-ack"
                    checked={guardianAck}
                    onChange={(e) => setGuardianAck(e.target.checked)}
                    className="mt-0.5 accent-indigo-500"
                  />
                  {GUARDIAN_ACK_LABEL}
                </label>
              </div>
            )}

            {store.error && (
              <p className="mt-2 text-sm text-red-300" role="alert">
                {store.error}
              </p>
            )}

            <div className="mt-3 flex items-center gap-3">
              {underThirteen ? (
                <button
                  type="button"
                  data-testid="consent-accept-parent"
                  disabled={!guardianAck || store.working}
                  onClick={() => void accept("parent")}
                  className="rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
                >
                  Agree and join (parent/guardian)
                </button>
              ) : (
                <button
                  type="button"
                  data-testid="consent-accept"
                  disabled={store.working}
                  onClick={() => void accept("student")}
                  className="rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
                >
                  I agree &mdash; join classroom
                </button>
              )}
              <button
                type="button"
                data-testid="consent-cancel"
                onClick={resetFlow}
                className="text-sm text-gray-400 hover:text-gray-200"
              >
                Cancel &mdash; don&rsquo;t join
              </button>
            </div>
          </div>
        )}

        {/* Step: the dashboard-sync prompt (T2 §3's anticipated ask). */}
        {step === "syncPrompt" && (
          <div className="mt-3" data-testid="sync-prompt">
            <p className="font-medium text-white">
              You&rsquo;re enrolled. Share your practice?
            </p>
            <p className="mt-1 text-sm text-gray-300">
              Right now, nothing leaves this device — your teacher will see you
              as practicing offline. Turning sharing on enables Cloud sync and
              dashboard sharing (exactly the list you just read). You can change
              either switch any time in Connections &amp; Privacy.
            </p>
            <div className="mt-3 flex items-center gap-3">
              <button
                type="button"
                data-testid="sync-prompt-accept"
                onClick={enableSharing}
                className="rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white hover:bg-indigo-500"
              >
                Turn on sharing
              </button>
              <button
                type="button"
                data-testid="sync-prompt-decline"
                onClick={resetFlow}
                className="text-sm text-gray-400 hover:text-gray-200"
              >
                Not now &mdash; keep everything on this device
              </button>
            </div>
          </div>
        )}
      </div>

      {/* ── Teacher side: minimal "My classroom" card ── */}
      {isTeacher && (
        <div className="rounded-lg bg-gray-800 p-4" data-testid="teacher-card">
          <h3 className="font-medium text-white">My classroom</h3>

          {store.classrooms.length === 0 && (
            <p className="mt-2 text-sm text-gray-400">
              No classroom yet. Create one, then hand its join code to your
              students — each student sees a consent screen before anything is
              shared.
            </p>
          )}

          <div className="mt-3 space-y-3">
            {store.classrooms.map((c) => {
              const expires = c.join_code_expires_at
                ? new Date(c.join_code_expires_at)
                : null;
              const live =
                c.join_code && expires && expires.getTime() > Date.now();
              const count = store.rosterCounts[c.id] ?? 0;
              return (
                <div
                  key={c.id}
                  className="rounded-md border border-gray-700 p-3"
                  data-testid={`classroom-${c.id}`}
                >
                  <div className="flex items-center justify-between gap-4">
                    <p className="font-medium text-white">{c.name}</p>
                    <p className="text-sm text-gray-400">
                      {count} active student{count === 1 ? "" : "s"}
                    </p>
                  </div>
                  <div className="mt-2 flex items-center justify-between gap-4">
                    <p className="text-sm text-gray-300">
                      {live ? (
                        <>
                          Join code{" "}
                          <span
                            className="font-mono tracking-widest text-white"
                            data-testid={`join-code-${c.id}`}
                          >
                            {c.join_code}
                          </span>{" "}
                          <span className="text-gray-400">
                            &mdash; expires {formatDate(c.join_code_expires_at)}
                          </span>
                        </>
                      ) : c.join_code ? (
                        "Join code expired."
                      ) : (
                        "No join code yet."
                      )}
                    </p>
                    <button
                      type="button"
                      data-testid={`issue-code-${c.id}`}
                      disabled={store.working}
                      onClick={() => void store.issueJoinCode(userId, c.id)}
                      className="shrink-0 rounded-md border border-gray-600 px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700 disabled:opacity-50"
                    >
                      New join code
                    </button>
                  </div>
                </div>
              );
            })}
          </div>

          <div className="mt-3 flex items-center gap-2">
            <input
              type="text"
              aria-label="New classroom name"
              placeholder="Classroom name"
              value={classroomName}
              onChange={(e) => setClassroomName(e.target.value)}
              className="w-56 rounded-md border border-gray-600 bg-gray-900 px-3 py-2 text-sm text-white placeholder-gray-500"
            />
            <button
              type="button"
              data-testid="create-classroom"
              disabled={!classroomName.trim() || store.working}
              onClick={() => {
                void store
                  .createClassroom(userId, classroomName)
                  .then((ok) => ok && setClassroomName(""));
              }}
              className="rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
            >
              Create classroom
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
