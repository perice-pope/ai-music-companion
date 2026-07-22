import type { ReactNode } from "react";
import type { DashboardClient } from "../lib/supabase";
import { getOwnProfile } from "../lib/queries";
import { useAsync } from "../lib/useAsync";

/**
 * The teacher gate (spec AC1). Reads ONLY the signed-in user's own profile
 * (policy profiles_select_own, migration 0001; grant 0007 L45) and renders
 * children only for role='teacher'. For any other role it renders a polite
 * dead-end and issues no further queries — students/parents have no
 * surface here (founder decision: no parent view).
 *
 * Note this gate is a UX convenience, not the security boundary: even a
 * client hacked past it would see nothing, because every fact/dim read is
 * admitted by the 0006 teacher policies server-side (RLS), not by this
 * component.
 */
export function TeacherGate({
  client,
  userId,
  onSignOut,
  children,
}: {
  client: DashboardClient;
  userId: string;
  onSignOut: () => void;
  children: ReactNode;
}) {
  const profile = useAsync(() => getOwnProfile(client, userId), [userId]);

  if (profile.status === "loading") {
    return <p className="mt-24 text-center text-slate-500">Loading…</p>;
  }
  if (profile.status === "error") {
    return (
      <p role="alert" className="mt-24 text-center text-red-600">
        Could not load your profile: {profile.message}
      </p>
    );
  }
  if (profile.data?.role !== "teacher") {
    return (
      <main className="mx-auto mt-24 max-w-md text-center">
        <h1 className="text-lg font-semibold text-slate-900">
          This dashboard is for teachers
        </h1>
        <p className="mt-2 text-sm text-slate-600">
          Your account isn&apos;t a teacher account. Students see their own
          practice history right in the AI Music Companion app — it&apos;s yours
          and it stays on your device.
        </p>
        <button
          onClick={onSignOut}
          className="mt-6 rounded border border-slate-300 px-4 py-2 text-sm hover:bg-slate-100"
        >
          Sign out
        </button>
      </main>
    );
  }
  return <>{children}</>;
}
