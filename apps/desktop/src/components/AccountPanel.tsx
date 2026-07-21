import { useEffect, useState } from "react";
import { useAuthStore } from "../stores/authStore";
import { useConnectionsStore } from "../stores/connectionsStore";
import { useSyncStore } from "../stores/syncStore";

/**
 * Optional cloud-sync account panel. Lives on the History page — signing in
 * is entirely opt-in and never blocks practice. When signed in, completed
 * sessions sync up to Supabase (Phase 3, Teacher Dashboard track).
 *
 * This component owns auth bootstrapping (`init` on mount) and kicks a sync
 * whenever a user is present, so we keep Supabase out of the always-on app
 * shell and the offline practice loop.
 */
export default function AccountPanel() {
  const { status, user, error, init, signIn, signUp, signOut, clearError } =
    useAuthStore();
  const sync = useSyncStore();
  const cloudSyncEnabled = useConnectionsStore((s) => s.cloudSyncEnabled);
  const dashboardSyncEnabled = useConnectionsStore(
    (s) => s.dashboardSyncEnabled,
  );

  const [mode, setMode] = useState<"sign_in" | "sign_up">("sign_in");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");

  // Restore any persisted session once on mount.
  useEffect(() => {
    init();
  }, [init]);

  // Push local sessions whenever we have a signed-in user AND cloud sync is
  // enabled (initial sign-in, session restore on boot, or a revisit that may
  // have new sessions). Gated by cloudSyncEnabled to respect opt-in.
  const userId = user?.id;
  const runSync = sync.syncAll;
  useEffect(() => {
    runSync(userId, cloudSyncEnabled);
  }, [userId, cloudSyncEnabled, runSync]);

  // #449 T2: the dashboard projection rides the SAME trigger (post-session,
  // never the audio path) behind its OWN additional opt-in. With the toggle
  // off this is a hard no-op — the legacy push above keeps its behavior and
  // nothing else leaves the device.
  const runDashboardSync = sync.syncDashboard;
  useEffect(() => {
    void runDashboardSync(userId, cloudSyncEnabled, dashboardSyncEnabled);
  }, [userId, cloudSyncEnabled, dashboardSyncEnabled, runDashboardSync]);

  if (status === "loading") {
    return (
      <div className="rounded-lg bg-gray-800 p-4 text-sm text-gray-400">
        Checking sign-in…
      </div>
    );
  }

  if (status === "signed_in" && user) {
    return (
      <div className="rounded-lg bg-gray-800 p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="text-sm text-gray-400">Signed in as</p>
            <p className="font-medium text-white">{user.email}</p>
          </div>
          <button
            onClick={() => signOut()}
            className="rounded-md border border-gray-600 px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700"
          >
            Sign out
          </button>
        </div>
        <p className="mt-3 text-sm text-gray-400" role="status">
          {sync.status === "syncing" && "Syncing your sessions…"}
          {sync.status === "synced" &&
            (sync.syncedThisRun > 0
              ? `Synced ${sync.syncedThisRun} session${sync.syncedThisRun === 1 ? "" : "s"}.`
              : "All sessions up to date.")}
          {sync.status === "error" && (
            <span className="text-red-300">Sync failed: {sync.error}</span>
          )}
          {sync.status === "idle" && "Sync ready."}
        </p>
      </div>
    );
  }

  // Signed out — show the auth form.
  const working = status === "working";
  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    clearError();
    if (mode === "sign_in") {
      signIn(email, password);
    } else {
      signUp(email, password, displayName || undefined);
    }
  };

  return (
    <form onSubmit={submit} className="rounded-lg bg-gray-800 p-4">
      <p className="mb-3 text-sm text-gray-300">
        Sign in to sync your practice history across devices.
      </p>
      {mode === "sign_up" && (
        <input
          type="text"
          aria-label="Display name"
          placeholder="Display name (optional)"
          value={displayName}
          onChange={(e) => setDisplayName(e.target.value)}
          className="mb-2 w-full rounded-md border border-gray-600 bg-gray-900 px-3 py-2 text-sm text-white placeholder-gray-500"
        />
      )}
      <input
        type="email"
        required
        aria-label="Email"
        placeholder="Email"
        value={email}
        onChange={(e) => setEmail(e.target.value)}
        className="mb-2 w-full rounded-md border border-gray-600 bg-gray-900 px-3 py-2 text-sm text-white placeholder-gray-500"
      />
      <input
        type="password"
        required
        aria-label="Password"
        placeholder="Password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        className="mb-3 w-full rounded-md border border-gray-600 bg-gray-900 px-3 py-2 text-sm text-white placeholder-gray-500"
      />
      {error && (
        <p className="mb-3 text-sm text-red-300" role="alert">
          {error}
        </p>
      )}
      <div className="flex items-center justify-between gap-4">
        <button
          type="submit"
          disabled={working}
          className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
        >
          {working
            ? "Working…"
            : mode === "sign_in"
              ? "Sign in"
              : "Create account"}
        </button>
        <button
          type="button"
          onClick={() => {
            clearError();
            setMode(mode === "sign_in" ? "sign_up" : "sign_in");
          }}
          className="text-sm text-indigo-400 hover:text-indigo-300"
        >
          {mode === "sign_in"
            ? "Need an account? Create one"
            : "Have an account? Sign in"}
        </button>
      </div>
    </form>
  );
}
