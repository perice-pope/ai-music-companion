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
  {
    name: "Trumpet",
    family: "Brass",
    freqMinHz: 155,
    freqMaxHz: 988,
    emoji: "🎺",
    polyphonic: false,
  },
  // Trombone deliberately NOT 🎺 — that's a trumpet, not a trombone.
  // Unicode has no dedicated trombone glyph; 🎶 is a neutral fallback
  // until we ship custom SVG art for instruments. Keeps us honest
  // toward trombonists in the meantime. Mirror in profiles/trombone.json.
  {
    name: "Trombone",
    family: "Brass",
    freqMinHz: 58,
    freqMaxHz: 587,
    emoji: "🎶",
    polyphonic: false,
  },
  {
    name: "French Horn",
    family: "Brass",
    freqMinHz: 87,
    freqMaxHz: 880,
    emoji: "📯",
    polyphonic: false,
  },
  {
    name: "Voice",
    family: "Voice",
    freqMinHz: 82,
    freqMaxHz: 1047,
    emoji: "🎤",
    polyphonic: false,
  },
  {
    name: "Violin",
    family: "Strings",
    freqMinHz: 196,
    freqMaxHz: 2637,
    emoji: "🎻",
    polyphonic: false,
  },
  {
    name: "Cello",
    family: "Strings",
    freqMinHz: 65,
    freqMaxHz: 988,
    emoji: "🎻",
    polyphonic: false,
  },
  {
    name: "Flute",
    family: "Woodwind",
    freqMinHz: 262,
    freqMaxHz: 2093,
    emoji: "🪈",
    polyphonic: false,
  },
  {
    name: "Clarinet",
    family: "Woodwind",
    freqMinHz: 147,
    freqMaxHz: 1568,
    emoji: "🎷",
    polyphonic: false,
  },
  {
    name: "Piano",
    family: "Keyboard",
    freqMinHz: 27,
    freqMaxHz: 4186,
    emoji: "🎹",
    polyphonic: true,
  },
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
        next_session_suggestions: [
          "Launch the Tauri dev build to hear yourself",
        ],
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
    case "recognize_pdf_score":
      // OMR needs the bundled engine, which only exists in the native shell.
      throw new Error(
        "Reading sheet-music PDFs needs the desktop app — run `pnpm tauri dev`.",
      );
    case "get_reveal":
      // Reveals key off the real `perception` reading, which the browser
      // preview mocks as `null` — so this isn't reached in practice. Mock a
      // calm "nothing to reveal" so the command is never unhandled.
      return null;
    case "start_lesson":
    case "submit_drill":
      // Lessons need the real ears (drills are graded from the mic); the
      // preview can't run one. Match the backend's calm error instead of an
      // unmocked-command throw.
      throw new Error(
        "Guided lessons need the desktop app — run `pnpm tauri dev`.",
      );
    case "end_lesson":
    case "end_explore":
      return undefined;
    case "start_explore_variation":
    case "apply_variation_delta":
    case "explore_last_phrase":
    case "edit_explore_note":
    case "undo_explore_edit":
      throw new Error(
        "Exploring variations needs the desktop app — run `pnpm tauri dev`.",
      );
    case "get_sound_mirror":
      return { profile: null, sessions_seen: 0 };
    case "get_mastery_wheel":
      // Preview: an empty wheel (nothing practiced in the browser).
      return {
        cells: Array.from({ length: 12 }, (_, tonic) => ({
          tonic,
          state: "none",
          attempts: 0,
          best_accuracy: 0,
          scales: [],
        })),
        intonation_trend: "unknown",
        tone_trend: "unknown",
        total_owned: 0,
      };
    case "practice_suggestions":
      // #453 S3: no history accumulates in a browser preview — the honest
      // empty list keeps the coaching box silent instead of erroring.
      return [];
    case "record_reveal":
      // No reveals fire in the preview (see above), but keep the command
      // handled: report an unchanged, empty collection.
      return 0;
    // Tauri v2 event subsystem rides over invoke.
    case "plugin:event|listen":
      return Math.floor(Math.random() * 1_000_000);
    case "plugin:event|unlisten":
      return undefined;
    // The version badge (#384) asks the shell for the bundle version; there
    // is no bundle in a browser preview, so say so honestly.
    case "plugin:app|version":
      return "0.0.0-preview";
    default:
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
