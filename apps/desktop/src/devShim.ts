/**
 * Dev-only Tauri IPC shim.
 *
 * When the frontend runs under `pnpm tauri dev` it gets a real
 * `window.__TAURI_INTERNALS__` injected by the native shell, and
 * `invoke()` / `listen()` talk to Rust. When it runs under plain
 * Vite (e.g. the preview window in this tooling, or a designer
 * eyeballing the UI in a browser tab), that global is undefined and
 * every `invoke` call throws:
 *
 *   TypeError: Cannot read properties of undefined (reading 'invoke')
 *
 * This module installs a minimal mock so screens render, the
 * state-machine transitions fire, and you can click through the
 * flows. Live pitch, recordings, and persistence obviously can't be
 * faked — those surfaces get synthetic/empty data.
 *
 * Stripped from production builds by the `import.meta.env.DEV`
 * guard (Vite dead-code-eliminates the whole function body).
 */

// Families are title-case here to match the real `InstrumentInfo`
// contract emitted by `apps/desktop/src-tauri/src/commands.rs` —
// `InstrumentFamily::display_name()` returns "Brass", "Strings", etc.
// The UI's family-badge coloring keys off those exact strings, so
// lowercase values here would exercise a different render path in
// preview than the production app.
const MOCK_INSTRUMENTS = [
  { name: "Trumpet", family: "Brass", freqMinHz: 155, freqMaxHz: 988, emoji: "🎺" },
  // Trombone deliberately NOT 🎺 — that's a trumpet, not a trombone.
  // Unicode has no dedicated trombone glyph; 🎶 is a neutral fallback
  // until we ship custom SVG art for instruments. Keeps us honest
  // toward trombonists in the meantime. Mirror in profiles/trombone.json.
  { name: "Trombone", family: "Brass", freqMinHz: 58, freqMaxHz: 587, emoji: "🎶" },
  { name: "French Horn", family: "Brass", freqMinHz: 87, freqMaxHz: 880, emoji: "📯" },
  { name: "Voice", family: "Voice", freqMinHz: 82, freqMaxHz: 1047, emoji: "🎤" },
  { name: "Violin", family: "Strings", freqMinHz: 196, freqMaxHz: 2637, emoji: "🎻" },
  { name: "Cello", family: "Strings", freqMinHz: 65, freqMaxHz: 988, emoji: "🎻" },
  { name: "Flute", family: "Woodwind", freqMinHz: 262, freqMaxHz: 2093, emoji: "🪈" },
  { name: "Clarinet", family: "Woodwind", freqMinHz: 147, freqMaxHz: 1568, emoji: "🎷" },
  { name: "Piano", family: "Keyboard", freqMinHz: 27, freqMaxHz: 4186, emoji: "🎹" },
];

interface InvokeArgs {
  [key: string]: unknown;
}

/**
 * Shape of the `__TAURI_INTERNALS__` global. Only the subset this
 * shim needs to fake — the real internals object is much larger, but
 * `@tauri-apps/api` only touches these fields for `invoke()` and
 * `listen()`.
 */
interface DevTauriInternals {
  invoke: (cmd: string, args?: InvokeArgs) => Promise<unknown>;
  transformCallback: (cb: unknown, once?: boolean) => number;
  ipc: { postMessage: () => void };
  metadata: {
    currentWindow: { label: string };
    currentWebview: { label: string };
  };
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: DevTauriInternals;
    // Vite's transformCallback pattern: the Tauri API stores each
    // registered listener at `window._<id>` keyed by the integer id
    // handed back from `transformCallback`. Typed here so we don't
    // need `as any` to assign into the slot.
    [key: `_${number}`]: unknown;
  }
}

async function handleInvoke(cmd: string, args?: InvokeArgs): Promise<unknown> {
  // eslint-disable-next-line no-console
  console.debug("[devShim] invoke", cmd, args);
  switch (cmd) {
    case "list_instruments":
      return MOCK_INSTRUMENTS;
    case "start_practice_session":
      return `mock-session-${Date.now()}`;
    case "switch_instrument":
      return `mock-segment-${Date.now()}`;
    case "end_practice_session":
      return {
        overall_assessment:
          "(Browser preview — no real audio analysis. Run `pnpm tauri dev` for the full experience.)",
        strengths: ["UI renders", "State transitions fire"],
        areas_to_improve: ["Live pitch needs the Tauri native shell"],
        next_session_suggestions: ["Launch the Tauri dev build to hear yourself"],
        duration_secs: 0,
        phrase_count: 0,
        instrument: (args?.instrument as string) ?? "Unknown",
      };
    case "get_session_history":
      return [];
    case "get_practice_stats":
      return {
        total_sessions: 0,
        total_time_secs: 0,
        sessions_this_week: 0,
        avg_session_length_secs: 0,
        trend: "stable" as const,
      };
    case "get_session_detail":
      throw new Error("devShim: no sessions available in browser preview");
    // Tauri v2 event subsystem rides over invoke.
    case "plugin:event|listen":
      return Math.floor(Math.random() * 1_000_000);
    case "plugin:event|unlisten":
      return undefined;
    default:
      // eslint-disable-next-line no-console
      console.warn(`[devShim] unmocked command "${cmd}"`, args);
      throw new Error(`devShim: command "${cmd}" is not mocked`);
  }
}

/**
 * Install the mock IPC on `window.__TAURI_INTERNALS__` if we're in
 * dev and no real Tauri shell is present. Idempotent.
 */
export function installDevShimIfNeeded(): void {
  if (!import.meta.env.DEV) return;
  if (typeof window === "undefined") return;
  // Real Tauri already injected its internals — leave well alone.
  if (window.__TAURI_INTERNALS__) return;

  // eslint-disable-next-line no-console
  console.warn(
    "[devShim] No Tauri shell detected — installing mock IPC for browser preview.",
  );

  window.__TAURI_INTERNALS__ = {
    invoke: handleInvoke,
    // `listen()` calls transformCallback to register a JS callback
    // by id, then invokes `plugin:event|listen` with that id. In the
    // shim the events never fire, so we just hand out a throwaway id.
    transformCallback: (cb: unknown, _once?: boolean) => {
      const id = Math.floor(Math.random() * 1_000_000);
      window[`_${id}`] = cb;
      return id;
    },
    ipc: { postMessage: () => {} },
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main" },
    },
  };
}
