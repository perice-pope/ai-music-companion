import { useState } from "react";
import { useAuthStore } from "../stores/authStore";

/**
 * Email + password sign-in — the same Supabase auth the desktop app uses
 * (one identity; teachers sign in here with the account they created in
 * the app). No sign-up: the dashboard is a read surface, not an
 * onboarding one.
 */
export function SignIn() {
  const { signIn, status, error, clearError } = useAuthStore();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  return (
    <main className="mx-auto mt-24 w-full max-w-sm rounded-lg border border-slate-200 bg-white p-8 shadow-sm">
      <h1 className="text-lg font-semibold text-slate-900">
        Teacher Dashboard
      </h1>
      <p className="mt-1 text-sm text-slate-500">
        Sign in with your AI Music Companion teacher account.
      </p>
      <form
        className="mt-6 space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void signIn(email, password);
        }}
      >
        <label className="block text-sm">
          <span className="text-slate-700">Email</span>
          <input
            type="email"
            required
            autoComplete="email"
            value={email}
            onChange={(e) => {
              clearError();
              setEmail(e.target.value);
            }}
            className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
          />
        </label>
        <label className="block text-sm">
          <span className="text-slate-700">Password</span>
          <input
            type="password"
            required
            autoComplete="current-password"
            value={password}
            onChange={(e) => {
              clearError();
              setPassword(e.target.value);
            }}
            className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
          />
        </label>
        {error && (
          <p role="alert" className="text-sm text-red-600">
            {error}
          </p>
        )}
        <button
          type="submit"
          disabled={status === "working"}
          className="w-full rounded bg-slate-900 py-2 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-50"
        >
          {status === "working" ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </main>
  );
}
