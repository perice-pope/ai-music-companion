import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";

/**
 * The running app's version, read from the bundle itself (Tauri
 * `getVersion()`) so it can never drift from what the installer says — the
 * point of #384 is that testers can quote a number that identifies the build.
 * Renders nothing until the version is known and nothing at all where the
 * shell API is absent (browser preview): the badge is informational, never
 * an error state.
 */
export default function AppVersionBadge({
  className = "",
}: {
  className?: string;
}) {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const v = await getVersion();
        if (!cancelled && v) setVersion(v);
      } catch {
        // No shell (browser preview) or a failed IPC round-trip: stay silent.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (!version) return null;
  return (
    <p
      className={`text-xs text-gray-500 ${className}`.trim()}
      data-testid="app-version"
      aria-label="App version"
    >
      v{version}
    </p>
  );
}
