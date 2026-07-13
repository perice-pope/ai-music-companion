/**
 * Browser-injected mock of the Tauri IPC boundary + an outbound-network
 * guard, used by the GUI E2E suite (Option B: UI E2E against the Vite web
 * build with mocked IPC; the native Tauri shell is covered separately by
 * the Rust E2E harness in `crates/brain/tests/e2e_offline.rs`).
 *
 * Why a dedicated mock and NOT the app's own `devShim`:
 *  - `devShim` is `import.meta.env.DEV`-gated and dead-code-eliminated from
 *    the production bundle the E2E suite drives, so it isn't present.
 *  - The E2E mock must additionally *record* every IPC call and *block +
 *    record* any outbound HTTP, so the suite can assert the practice→recap
 *    flow needs no network. `devShim` does neither.
 *
 * Playwright runs `installTauriMockAndNetGuard` via `addInitScript` BEFORE
 * any app code, so `window.__TAURI_INTERNALS__` already exists by the time
 * the React tree mounts and the app's `installDevShimIfNeeded()` sees it and
 * leaves it alone (it early-returns when internals are present). No app
 * source is touched.
 *
 * `makeIpcHandler` factors the seeded IPC logic out as a pure function so the
 * same behaviour can be unit-checked under Node/Vitest without a browser —
 * see `e2e/tests/tauri-mock.unit.test.ts`. The browser-injected function
 * below intentionally inlines an equivalent handler because `addInitScript`
 * serialises a single self-contained function and cannot close over module
 * scope.
 */

/** A recorded IPC invocation. */
export interface RecordedInvoke {
  cmd: string;
  args: Record<string, unknown> | undefined;
}

/** A recorded (blocked) outbound network attempt. */
export interface RecordedNetwork {
  kind: "fetch" | "xhr" | "websocket" | "beacon";
  url: string;
}

declare global {
  interface Window {
    /** Ordered log of every IPC command the app invoked. */
    __ipcCalls: RecordedInvoke[];
    /** Ordered log of every outbound network attempt (all blocked). */
    __netCalls: RecordedNetwork[];
    /**
     * How long the seeded `import_audio_file` pends before returning, in ms.
     * Defaults to 0 — an instant backend, the exact shape three VA runs of
     * #336 could never see a loading message in. Tests set it to simulate a
     * slower transcription.
     */
    __importDelayMs?: number;
    __TAURI_INTERNALS__?: unknown;
  }
}

/**
 * Instruments the seeded backend returns. Mirrors the camelCase
 * `InstrumentInfo` wire contract (`commands::InstrumentInfo`,
 * `rename_all = "camelCase"`). Small but real-shaped.
 */
export const SEEDED_INSTRUMENTS = [
  {
    name: "Trumpet",
    family: "Brass",
    freqMinHz: 155,
    freqMaxHz: 988,
    vibratoToleranceCents: 15,
    emoji: "🎺",
  },
  {
    name: "Voice",
    family: "Voice",
    freqMinHz: 82,
    freqMaxHz: 1047,
    vibratoToleranceCents: 30,
    emoji: "🎤",
  },
  {
    name: "Violin",
    family: "Strings",
    freqMinHz: 196,
    freqMaxHz: 2637,
    vibratoToleranceCents: 20,
    emoji: "🎻",
  },
];

/**
 * Recap returned from `end_practice_session`. Shaped to
 * `brain::session::SessionRecap` (frontend `SessionRecap` type).
 * `phrase_count > 0` so the recap renders the full summary variant
 * (strengths before areas — the product invariant the UI enforces),
 * not the empty state.
 */
export const SEEDED_RECAP = {
  overall_assessment:
    "Solid, focused session — your sound stayed centered and your phrasing was musical.",
  strengths: ["Steady air support", "Clean attacks on the upper register"],
  areas_to_improve: ["Watch the tuning on sustained high notes"],
  next_session_suggestions: ["Try long tones with a drone for intonation"],
  duration_secs: 312,
  phrase_count: 7,
  instrument: "Trumpet",
  fingerprint: {
    intonation: {
      note_count: 64,
      mean_cents: 3,
      mean_abs_cents: 8,
      in_tune_ratio: 0.82,
      tendencies: [],
    },
    groove: {
      tempo_bpm: 96,
      swing_ratio: null,
      mean_ioi_secs: 0.62,
      timing_consistency: 0.88,
      onset_count: 120,
    },
  },
};

/**
 * Duration-aware empty-state recap (`brain::session::SessionRecap` with
 * `phrase_count: 0`). Mirrors what the backend's `empty_state_recap` returns
 * for a session that ran long enough that the user clearly *was* there — a
 * calm "couldn't pick out phrases, check your mic" note, never "you didn't
 * play" (#185). The seeded backend returns this for a **Voice** session so the
 * suite can exercise empty-recap rendering + the recap's dark surface (#186).
 */
export const SEEDED_EMPTY_RECAP = {
  overall_assessment:
    "I couldn't quite pick out distinct phrases this time. If you were playing, try moving a little closer to the mic or nudging your input level up — then come back and I'll listen again.",
  strengths: [],
  areas_to_improve: [],
  next_session_suggestions: [
    "Check your microphone input level, then play a few clear, sustained notes.",
  ],
  duration_secs: 312,
  phrase_count: 0,
  instrument: "Voice",
  fingerprint: null,
  idiom_notes: [],
  connections: [],
};

/** A seeded library score (`brain::store::ScoreLibraryEntry` wire shape). */
export const SEEDED_SCORE = {
  id: "00000000-0000-0000-0000-0000000000aa",
  title: "Seeded Etude",
  composer: "E2E",
  source_filename: "seeded-etude.musicxml",
  added_at: "2026-06-12T00:00:00Z",
  last_practiced_at: null,
  part_index: 0,
  duration_measures: 8,
};

/** Minimal valid MusicXML returned by `get_score` for the seeded score. */
export const SEEDED_MUSICXML =
  '<?xml version="1.0"?><score-partwise version="4.0"><part-list>' +
  '<score-part id="P1"><part-name>Music</part-name></score-part></part-list>' +
  "<part id=\"P1\"><measure number=\"1\"><attributes><divisions>1</divisions>" +
  "<key><fifths>0</fifths></key><time><beats>4</beats><beat-type>4</beat-type></time>" +
  "<clef><sign>G</sign><line>2</line></clef></attributes>" +
  "<note><pitch><step>C</step><octave>4</octave></pitch><duration>4</duration>" +
  "<type>whole</type></note></measure></part></score-partwise>";

/**
 * Build the seeded IPC handler. Same contract the app's `invoke()` expects:
 * `(cmd, args) => Promise<unknown>`. Every call is appended to `ipcCalls`
 * first. Pure apart from that log, so it can be exercised directly in a Node
 * unit test.
 */
export function makeIpcHandler(ipcCalls: RecordedInvoke[]) {
  let sessionSeq = 0;
  // Track the instrument the current session started with so the recap can vary
  // (Voice → empty-state recap, exercising the #185/#186 rendering path).
  let lastInstrument: string | undefined;
  return async function handleInvoke(
    cmd: string,
    args?: Record<string, unknown>,
  ): Promise<unknown> {
    ipcCalls.push({ cmd, args });
    switch (cmd) {
      case "list_instruments":
        return SEEDED_INSTRUMENTS;
      case "start_practice_session":
        sessionSeq += 1;
        lastInstrument = args?.instrument as string | undefined;
        return `e2e-session-${sessionSeq}`;
      case "switch_instrument":
        lastInstrument = (args?.name as string | undefined) ?? lastInstrument;
        return `e2e-segment-${Date.now()}`;
      case "end_practice_session":
        // A Voice session returns the duration-aware empty-state recap so the
        // suite can drive empty-recap rendering (#185/#186); every other
        // instrument returns the full summary recap.
        return lastInstrument === "Voice" ? SEEDED_EMPTY_RECAP : SEEDED_RECAP;
      case "get_session_history":
        return [];
      case "list_scores":
        return [SEEDED_SCORE];
      case "get_score":
        return { entry: SEEDED_SCORE, music_xml: SEEDED_MUSICXML };
      // Audio → transcription import (#336). Pends for `__importDelayMs`
      // (default 0 — an instant backend) then returns a clean monophonic
      // result echoing the dropped file's name, shaped to `ImportedAudioDto`.
      case "import_audio_file": {
        const delayMs =
          (globalThis as { __importDelayMs?: number }).__importDelayMs ?? 0;
        if (delayMs > 0) {
          await new Promise((resolve) => setTimeout(resolve, delayMs));
        }
        return {
          entry: {
            ...SEEDED_SCORE,
            id: "00000000-0000-0000-0000-0000000000bb",
            title: "Transcribed Recording",
            source_filename:
              (args?.sourceFilename as string | undefined) ?? "recording.wav",
            duration_measures: 4,
          },
          note_count: 8,
          mean_confidence: 0.91,
          polyphony: 0.0,
          polyphonic: false,
          low_confidence: false,
        };
      }
      case "get_practice_stats":
        return {
          total_sessions: 0,
          total_time_secs: 0,
          sessions_this_week: 0,
          avg_session_length_secs: 0,
          trend: "stable",
        };
      // Tauri v2 routes its event subsystem over invoke. The seeded backend
      // never emits live events (no mic headless), so listen/unlisten just
      // hand back / accept a throwaway id.
      case "plugin:event|listen":
        return Math.floor(Math.random() * 1_000_000);
      case "plugin:event|unlisten":
        return undefined;
      default:
        // Unknown command — reject so a regression that adds a new IPC
        // dependency to the core offline flow surfaces loudly.
        throw new Error(`[e2e tauri-mock] unmocked command "${cmd}"`);
    }
  };
}

/**
 * Injected into the page before app code runs. It:
 *  1. installs the recording mock IPC on `window.__TAURI_INTERNALS__`, and
 *  2. wraps `fetch` / `XMLHttpRequest` / `WebSocket` / `sendBeacon` so any
 *     outbound network attempt is recorded AND blocked. The offline
 *     guarantee is then asserted by checking `window.__netCalls` is empty
 *     (Playwright also aborts non-local requests at the route layer as a
 *     second, independent guard).
 *
 * Self-contained: it must not reference any module-scope binding, because
 * `addInitScript` serialises only this function body into the page.
 */
export function installTauriMockAndNetGuard(): void {
  const ipcCalls: RecordedInvoke[] = [];
  const netCalls: RecordedNetwork[] = [];
  window.__ipcCalls = ipcCalls;
  window.__netCalls = netCalls;

  let sessionSeq = 0;
  let lastInstrument: string | undefined;
  const instruments = [
    {
      name: "Trumpet",
      family: "Brass",
      freqMinHz: 155,
      freqMaxHz: 988,
      vibratoToleranceCents: 15,
      emoji: "🎺",
    },
    {
      name: "Voice",
      family: "Voice",
      freqMinHz: 82,
      freqMaxHz: 1047,
      vibratoToleranceCents: 30,
      emoji: "🎤",
    },
    {
      name: "Violin",
      family: "Strings",
      freqMinHz: 196,
      freqMaxHz: 2637,
      vibratoToleranceCents: 20,
      emoji: "🎻",
    },
  ];
  const recap = {
    overall_assessment:
      "Solid, focused session — your sound stayed centered and your phrasing was musical.",
    strengths: ["Steady air support", "Clean attacks on the upper register"],
    areas_to_improve: ["Watch the tuning on sustained high notes"],
    next_session_suggestions: ["Try long tones with a drone for intonation"],
    duration_secs: 312,
    phrase_count: 7,
    instrument: "Trumpet",
    fingerprint: {
      intonation: {
        note_count: 64,
        mean_cents: 3,
        mean_abs_cents: 8,
        in_tune_ratio: 0.82,
        tendencies: [],
      },
      groove: {
        tempo_bpm: 96,
        swing_ratio: null,
        mean_ioi_secs: 0.62,
        timing_consistency: 0.88,
        onset_count: 120,
      },
    },
  };
  // Duration-aware empty-state recap (phrase_count 0) — returned for a Voice
  // session so the suite can drive empty-recap rendering (#185/#186).
  const emptyRecap = {
    overall_assessment:
      "I couldn't quite pick out distinct phrases this time. If you were playing, try moving a little closer to the mic or nudging your input level up — then come back and I'll listen again.",
    strengths: [],
    areas_to_improve: [],
    next_session_suggestions: [
      "Check your microphone input level, then play a few clear, sustained notes.",
    ],
    duration_secs: 312,
    phrase_count: 0,
    instrument: "Voice",
    fingerprint: null,
    idiom_notes: [],
    connections: [],
  };
  const score = {
    id: "00000000-0000-0000-0000-0000000000aa",
    title: "Seeded Etude",
    composer: "E2E",
    source_filename: "seeded-etude.musicxml",
    added_at: "2026-06-12T00:00:00Z",
    last_practiced_at: null,
    part_index: 0,
    duration_measures: 8,
  };
  const musicXml =
    '<?xml version="1.0"?><score-partwise version="4.0"><part-list>' +
    '<score-part id="P1"><part-name>Music</part-name></score-part></part-list>' +
    '<part id="P1"><measure number="1"><attributes><divisions>1</divisions>' +
    '<key><fifths>0</fifths></key><time><beats>4</beats><beat-type>4</beat-type></time>' +
    '<clef><sign>G</sign><line>2</line></clef></attributes>' +
    '<note><pitch><step>C</step><octave>4</octave></pitch><duration>4</duration>' +
    "<type>whole</type></note></measure></part></score-partwise>";

  async function handleInvoke(
    cmd: string,
    args?: Record<string, unknown>,
  ): Promise<unknown> {
    ipcCalls.push({ cmd, args });
    switch (cmd) {
      case "list_instruments":
        return instruments;
      case "start_practice_session":
        sessionSeq += 1;
        lastInstrument = args?.instrument as string | undefined;
        return `e2e-session-${sessionSeq}`;
      case "switch_instrument":
        lastInstrument = (args?.name as string | undefined) ?? lastInstrument;
        return `e2e-segment-${Date.now()}`;
      case "end_practice_session":
        return lastInstrument === "Voice" ? emptyRecap : recap;
      case "get_session_history":
        return [];
      case "list_scores":
        return [score];
      case "get_score":
        return { entry: score, music_xml: musicXml };
      // Audio → transcription import (#336): pends for `__importDelayMs`
      // (default 0 — an instant backend), mirroring the exported handler.
      case "import_audio_file": {
        const delayMs = window.__importDelayMs ?? 0;
        if (delayMs > 0) {
          await new Promise((resolve) => setTimeout(resolve, delayMs));
        }
        return {
          entry: {
            ...score,
            id: "00000000-0000-0000-0000-0000000000bb",
            title: "Transcribed Recording",
            source_filename:
              (args?.sourceFilename as string | undefined) ?? "recording.wav",
            duration_measures: 4,
          },
          note_count: 8,
          mean_confidence: 0.91,
          polyphony: 0.0,
          polyphonic: false,
          low_confidence: false,
        };
      }
      case "get_practice_stats":
        return {
          total_sessions: 0,
          total_time_secs: 0,
          sessions_this_week: 0,
          avg_session_length_secs: 0,
          trend: "stable",
        };
      case "plugin:event|listen":
        return Math.floor(Math.random() * 1_000_000);
      case "plugin:event|unlisten":
        return undefined;
      default:
        throw new Error(`[e2e tauri-mock] unmocked command "${cmd}"`);
    }
  }

  window.__TAURI_INTERNALS__ = {
    invoke: handleInvoke,
    transformCallback: (cb: unknown) => {
      const id = Math.floor(Math.random() * 1_000_000);
      (window as unknown as Record<string, unknown>)[`_${id}`] = cb;
      return id;
    },
    ipc: { postMessage: () => {} },
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main" },
    },
  };

  // --- Outbound-network guard --------------------------------------------
  // Any HTTP/WS the practice flow attempts is a bug in the offline promise.
  // Record + block it here (in addition to Playwright's route-level abort).
  window.fetch = ((input: RequestInfo | URL) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : (input as Request).url;
    netCalls.push({ kind: "fetch", url: String(url) });
    return Promise.reject(new Error(`[e2e net-guard] blocked fetch → ${url}`));
  }) as typeof window.fetch;

  const RealXHR = window.XMLHttpRequest;
  class GuardedXHR extends RealXHR {
    open(method: string, url: string | URL): void {
      netCalls.push({ kind: "xhr", url: String(url) });
      // Redirect at a target that can never leave the page.
      super.open(method, "data:,blocked");
    }
  }
  window.XMLHttpRequest = GuardedXHR as unknown as typeof XMLHttpRequest;

  if (typeof window.WebSocket === "function") {
    window.WebSocket = function (url: string | URL) {
      netCalls.push({ kind: "websocket", url: String(url) });
      throw new Error(`[e2e net-guard] blocked WebSocket → ${url}`);
    } as unknown as typeof WebSocket;
  }

  if (navigator.sendBeacon) {
    navigator.sendBeacon = ((url: string | URL) => {
      netCalls.push({ kind: "beacon", url: String(url) });
      return false;
    }) as typeof navigator.sendBeacon;
  }
}
