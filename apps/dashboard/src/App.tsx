import { useEffect } from "react";
import { supabase, type DashboardClient } from "./lib/supabase";
import { listClassrooms } from "./lib/queries";
import { useAsync } from "./lib/useAsync";
import { useAuthStore } from "./stores/authStore";
import { useNavStore } from "./stores/navStore";
import { SignIn } from "./components/SignIn";
import { TeacherGate } from "./components/TeacherGate";
import { RosterHeat } from "./components/RosterHeat";
import { StudentDrilldown } from "./components/StudentDrilldown";
import { IntegrityPanel } from "./components/IntegrityPanel";

/**
 * Shell: sign-in → teacher gate → per-classroom surfaces (roster heat /
 * drill-down / integrity panel). The client is passed down as a prop so
 * tests can mount every surface against a mock routed by relation name.
 */
export default function App({
  client = supabase,
}: {
  client?: DashboardClient;
}) {
  const { status, user, init, signOut } = useAuthStore();

  useEffect(() => {
    void init();
  }, [init]);

  if (status === "loading") {
    return <p className="mt-24 text-center text-slate-500">Loading…</p>;
  }
  if (status === "signed_out" || status === "working" || !user) {
    return <SignIn />;
  }
  return (
    <TeacherGate
      client={client}
      userId={user.id}
      onSignOut={() => void signOut()}
    >
      <Shell
        client={client}
        email={user.email ?? ""}
        onSignOut={() => void signOut()}
      />
    </TeacherGate>
  );
}

function Shell({
  client,
  email,
  onSignOut,
}: {
  client: DashboardClient;
  email: string;
  onSignOut: () => void;
}) {
  const {
    classroomId,
    view,
    selectClassroom,
    openStudent,
    showRoster,
    showIntegrity,
  } = useNavStore();

  const classrooms = useAsync(() => listClassrooms(client), []);

  useEffect(() => {
    if (
      classrooms.status === "ready" &&
      classroomId === null &&
      classrooms.data.length > 0
    ) {
      selectClassroom(classrooms.data[0].id);
    }
  }, [classrooms, classroomId, selectClassroom]);

  return (
    <div className="mx-auto max-w-6xl px-6 py-6">
      <header className="mb-6 flex flex-wrap items-center gap-4 border-b border-slate-200 pb-4">
        <h1 className="text-lg font-semibold text-slate-900">
          Practice Dashboard
        </h1>
        {classrooms.status === "ready" && classrooms.data.length > 0 && (
          <select
            aria-label="Classroom"
            value={classroomId ?? ""}
            onChange={(e) => selectClassroom(e.target.value)}
            className="rounded border border-slate-300 px-2 py-1 text-sm"
          >
            {classrooms.data.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        )}
        <nav className="flex gap-2 text-sm">
          <TabButton
            active={view.kind !== "integrity"}
            onClick={showRoster}
            label="Roster"
          />
          <TabButton
            active={view.kind === "integrity"}
            onClick={showIntegrity}
            label="Integrity"
          />
        </nav>
        <span className="ml-auto text-xs text-slate-400">{email}</span>
        <button
          onClick={onSignOut}
          className="rounded border border-slate-300 px-3 py-1 text-sm hover:bg-slate-100"
        >
          Sign out
        </button>
      </header>

      {classrooms.status === "loading" && (
        <p className="text-slate-500">Loading classrooms…</p>
      )}
      {classrooms.status === "error" && (
        <p role="alert" className="text-red-600">
          Could not load classrooms: {classrooms.message}
        </p>
      )}
      {classrooms.status === "ready" && classrooms.data.length === 0 && (
        <p className="text-slate-500">
          No classrooms yet. Create one in the AI Music Companion app, then
          share its join code with your students.
        </p>
      )}

      {classroomId !== null && (
        <>
          {view.kind === "roster" && (
            <RosterHeat
              client={client}
              classroomId={classroomId}
              onOpenStudent={openStudent}
            />
          )}
          {view.kind === "student" && (
            <StudentDrilldown
              client={client}
              studentId={view.studentId}
              displayName={view.displayName}
              onBack={showRoster}
            />
          )}
          {view.kind === "integrity" && (
            <IntegrityPanel client={client} classroomId={classroomId} />
          )}
        </>
      )}
    </div>
  );
}

function TabButton({
  active,
  onClick,
  label,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded px-3 py-1 ${
        active ? "bg-slate-900 text-white" : "text-slate-600 hover:bg-slate-100"
      }`}
    >
      {label}
    </button>
  );
}
