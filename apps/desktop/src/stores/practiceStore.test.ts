import { describe, it, expect, beforeEach, vi } from "vitest";
import type {
  CoachingTip,
  PhraseSummary,
  Reveal,
  SessionRecap,
} from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Minimal localStorage polyfill for jsdom with explicit tracking.
const store: Record<string, string> = {};
const localStorageMock = {
  getItem: vi.fn((k: string) => store[k] ?? null),
  setItem: vi.fn((k: string, v: string) => {
    store[k] = v;
  }),
  removeItem: vi.fn((k: string) => {
    delete store[k];
  }),
  clear: vi.fn(() => {
    for (const k of Object.keys(store)) delete store[k];
  }),
  get length() {
    return Object.keys(store).length;
  },
  key: vi.fn((_i: number) => null),
};
Object.defineProperty(window, "localStorage", { value: localStorageMock });

// Import AFTER mocks are set up so the store's initial coachingEnabled
// read pulls from our mocked localStorage.
async function freshStore() {
  vi.resetModules();
  const mod = await import("./practiceStore");
  return mod.usePracticeStore;
}

describe("practiceStore — state machine", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("starts in the selector screen with idle status", async () => {
    const useStore = await freshStore();
    const s = useStore.getState();
    expect(s.screen).toBe("selector");
    expect(s.status).toBe("idle");
    expect(s.sessionId).toBeNull();
    expect(s.phrases).toEqual([]);
  });

  it("startSession moves idle → listening and navigates to session screen", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("session-abc-123");

    await useStore.getState().startSession("Trumpet");

    const s = useStore.getState();
    expect(s.status).toBe("listening");
    expect(s.screen).toBe("session");
    expect(s.sessionId).toBe("session-abc-123");
    expect(s.instrumentName).toBe("Trumpet");
    expect(s.startedAtMs).not.toBeNull();
    // invoke called with the right command + args. Coaching narration is
    // off by default (offline-first), so the session starts with it disabled.
    expect(mockInvoke).toHaveBeenCalledWith("start_practice_session", {
      instrument: "Trumpet",
      practiceMode: "practice",
      coachingEnabled: false,
      scoreId: null,
    });
  });

  it("startSession is rejected when already listening", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");

    await useStore.getState().startSession("Trumpet");
    expect(useStore.getState().status).toBe("listening");

    // Second start must throw — and must NOT call invoke a second time.
    await expect(useStore.getState().startSession("Piano")).rejects.toThrow(
      /cannot start session from status=listening/,
    );
    // Only the first call reached invoke.
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it("endSession is rejected from idle", async () => {
    const useStore = await freshStore();
    await expect(useStore.getState().endSession()).rejects.toThrow(
      /cannot end session from status=idle/,
    );
    // invoke must never fire for a refused transition.
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("endSession happy path produces recap and navigates to recap screen", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");

    const recap: SessionRecap = {
      overall_assessment: "Nice work",
      strengths: ["A"],
      areas_to_improve: ["B"],
      next_session_suggestions: ["C"],
      duration_secs: 60,
      phrase_count: 3,
      instrument: "Trumpet",
    };
    mockInvoke.mockResolvedValueOnce(recap);

    await useStore.getState().endSession();

    const s = useStore.getState();
    expect(s.status).toBe("idle");
    expect(s.screen).toBe("recap");
    expect(s.recap).toEqual(recap);
    expect(s.recapError).toBeNull();
  });

  it("endSession failure still navigates to recap but sets recapError", async () => {
    // Design invariant: never fail loudly on a recap. The session
    // happened — the UI must still acknowledge it.
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");

    mockInvoke.mockRejectedValueOnce(new Error("llm timeout"));
    await useStore.getState().endSession();

    const s = useStore.getState();
    expect(s.status).toBe("idle");
    expect(s.screen).toBe("recap");
    expect(s.recap).toBeNull();
    expect(s.recapError).toContain("llm timeout");
  });

  it("returnToSelector resets transient session state", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");

    useStore.getState().returnToSelector();
    const s = useStore.getState();
    expect(s.screen).toBe("selector");
    expect(s.status).toBe("idle");
    expect(s.sessionId).toBeNull();
    expect(s.instrumentName).toBeNull();
    expect(s.phrases).toEqual([]);
  });

  it("mid-session instrument switch updates instrumentName and segmentId", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid"); // start
    await useStore.getState().startSession("Trumpet");

    mockInvoke.mockResolvedValueOnce("seg-piano-42");
    await useStore.getState().switchInstrument("Piano");

    const s = useStore.getState();
    expect(s.instrumentName).toBe("Piano");
    expect(s.segmentId).toBe("seg-piano-42");
    // Still listening — switch doesn't end the session.
    expect(s.status).toBe("listening");
    expect(mockInvoke).toHaveBeenLastCalledWith("switch_instrument", {
      instrument: "Piano",
      practiceMode: "practice",
    });
  });

  it("switchInstrument is rejected when not listening", async () => {
    const useStore = await freshStore();
    await expect(useStore.getState().switchInstrument("Piano")).rejects.toThrow(
      /cannot switch instrument from status=idle/,
    );
  });
});

describe("practiceStore — tick and timer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("tick advances elapsedSecs only while listening", async () => {
    const useStore = await freshStore();
    // Not listening → tick should be a no-op.
    useStore.getState().tick();
    expect(useStore.getState().elapsedSecs).toBe(0);

    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");

    // Force a known start time so elapsed is deterministic.
    useStore.setState({ startedAtMs: Date.now() - 3500 });
    useStore.getState().tick();
    // 3500ms → floor = 3s
    expect(useStore.getState().elapsedSecs).toBe(3);
  });

  it("tick does not advance while ending", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");
    useStore.setState({ status: "ending", startedAtMs: Date.now() - 10_000 });
    useStore.getState().tick();
    // Elapsed should not have moved off zero despite 10s of wall clock
    expect(useStore.getState().elapsedSecs).toBe(0);
  });
});

describe("practiceStore — tip queue", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("pushTip appends with unique ids and metadata", async () => {
    const useStore = await freshStore();
    const tip: CoachingTip = {
      text: "Nice tone",
      severity: "encouragement",
      category: "tone",
    };
    useStore.getState().pushTip(tip, 0);
    useStore.getState().pushTip(tip, 1);

    const queue = useStore.getState().tipQueue;
    expect(queue.length).toBe(2);
    expect(queue[0].id).not.toBe(queue[1].id);
    expect(queue[0].phraseIndex).toBe(0);
    expect(queue[1].phraseIndex).toBe(1);
  });

  it("dismissTip removes by id without touching siblings", async () => {
    const useStore = await freshStore();
    const tip: CoachingTip = {
      text: "x",
      severity: "suggestion",
      category: "tone",
    };
    useStore.getState().pushTip(tip, 0);
    useStore.getState().pushTip(tip, 1);
    const firstId = useStore.getState().tipQueue[0].id;

    useStore.getState().dismissTip(firstId);

    const queue = useStore.getState().tipQueue;
    expect(queue.length).toBe(1);
    expect(queue[0].id).not.toBe(firstId);
  });
});

describe("practiceStore — requestCoachingTip (live loop)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  function samplePhrase(phraseIndex = 0): PhraseSummary {
    return {
      phrase_index: phraseIndex,
      start_time: 0,
      end_time: 1,
      duration_secs: 1,
      note_count: 6,
      pitch_stats: {
        mean_hz: 440,
        min_hz: 430,
        max_hz: 450,
        range_cents: 80,
        pitches: [440, 440, 440],
      },
      dynamics: {
        mean_amplitude: 0.5,
        min_amplitude: 0.3,
        max_amplitude: 0.8,
        dynamic_range: 0.5,
      },
      stability: 0.8,
    };
  }

  const sampleTip: CoachingTip = {
    text: "Let the phrase breathe at the top.",
    severity: "suggestion",
    category: "tone",
  };

  async function listeningStore(coachingEnabled: boolean) {
    if (coachingEnabled) {
      localStorageMock.setItem("ai-music-companion:coaching-enabled", "true");
    }
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid"); // start_practice_session
    await useStore.getState().startSession("Trumpet");
    mockInvoke.mockClear();
    return useStore;
  }

  it("fires no IPC when coaching is disabled", async () => {
    const useStore = await listeningStore(false);
    await useStore.getState().requestCoachingTip(samplePhrase());
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(useStore.getState().tipQueue).toHaveLength(0);
  });

  it("fires no IPC when no session is listening", async () => {
    localStorageMock.setItem("ai-music-companion:coaching-enabled", "true");
    const useStore = await freshStore();
    // Never started a session → status is idle.
    await useStore.getState().requestCoachingTip(samplePhrase());
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("when enabled: gets a tip, surfaces it, and persists it", async () => {
    const useStore = await listeningStore(true);
    // get_coaching_tip → tip, then record_coaching_tip → ok
    mockInvoke.mockResolvedValueOnce(sampleTip);
    mockInvoke.mockResolvedValueOnce(undefined);

    await useStore.getState().requestCoachingTip(samplePhrase(2));

    // Asked the backend for a tip with phrase + context.
    expect(mockInvoke).toHaveBeenCalledWith(
      "get_coaching_tip",
      expect.objectContaining({
        phrase: expect.objectContaining({ phrase_index: 2 }),
        phrasesPlayed: expect.any(Number),
        sessionDurationSecs: expect.any(Number),
      }),
    );
    // Surfaced into the tip panel queue.
    const queue = useStore.getState().tipQueue;
    expect(queue).toHaveLength(1);
    expect(queue[0].tip.text).toBe(sampleTip.text);
    expect(queue[0].phraseIndex).toBe(2);
    // Persisted via record_coaching_tip.
    expect(mockInvoke).toHaveBeenCalledWith("record_coaching_tip", {
      phraseIndex: 2,
      tip: sampleTip,
    });
  });

  it("when the backend returns null (no tip): surfaces nothing, records nothing", async () => {
    const useStore = await listeningStore(true);
    mockInvoke.mockResolvedValueOnce(null); // get_coaching_tip → no tip

    await useStore.getState().requestCoachingTip(samplePhrase());

    expect(useStore.getState().tipQueue).toHaveLength(0);
    // Only get_coaching_tip was called — no record_coaching_tip.
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith(
      "get_coaching_tip",
      expect.anything(),
    );
  });

  it("a failed tip request never throws and leaves the queue empty", async () => {
    const useStore = await listeningStore(true);
    mockInvoke.mockRejectedValueOnce(new Error("network down"));

    await expect(
      useStore.getState().requestCoachingTip(samplePhrase()),
    ).resolves.toBeUndefined();
    expect(useStore.getState().tipQueue).toHaveLength(0);
  });
});

describe("practiceStore — requestReveal (#253)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  function samplePhrase(phraseIndex = 0): PhraseSummary {
    return {
      phrase_index: phraseIndex,
      start_time: 0,
      end_time: 1,
      duration_secs: 1,
      note_count: 6,
      pitch_stats: {
        mean_hz: 440,
        min_hz: 430,
        max_hz: 450,
        range_cents: 80,
        pitches: [440],
      },
      dynamics: {
        mean_amplitude: 0.5,
        min_amplitude: 0.3,
        max_amplitude: 0.8,
        dynamic_range: 0.5,
      },
      stability: 0.8,
    };
  }

  const gDorianKey = {
    tonic: 7,
    mode: "dorian",
    name: "G Dorian",
    confidence: 0.9,
    alternative: null,
  };
  const perceptionWith = (key: typeof gDorianKey | null) => ({
    tempo_bpm: null,
    swing_ratio: null,
    locked: false,
    key,
  });
  const sampleReveal: Reveal = {
    concept: "G Dorian",
    connection: 'Miles Davis — "So What"',
    why: "Modal jazz.",
    source: "grounded",
    tonic: 7,
    mode: "dorian",
  };

  async function listeningStoreWithKey() {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid"); // start_practice_session
    await useStore.getState().startSession("Trumpet");
    useStore.setState({ perception: perceptionWith(gDorianKey) });
    mockInvoke.mockClear();
    return useStore;
  }

  // §6: only during a live session. A no-op when idle must fire NO IPC —
  // deleting the `status !== "listening"` guard would call get_reveal here.
  it("fires no IPC when no session is listening", async () => {
    const useStore = await freshStore();
    useStore.setState({ perception: perceptionWith(gDorianKey) });
    await useStore.getState().requestReveal(samplePhrase());
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(useStore.getState().revealQueue).toHaveLength(0);
  });

  // §6 "silence → no reveal": without a detected key there's nothing to reveal,
  // so no IPC fires. Deleting the `!perception?.key` guard breaks this.
  it("fires no IPC when perception has no key", async () => {
    const useStore = await listeningStoreWithKey();
    useStore.setState({ perception: perceptionWith(null) });
    await useStore.getState().requestReveal(samplePhrase());
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(useStore.getState().revealQueue).toHaveLength(0);
  });

  // The live key/mode/confidence are passed to the backend, and a returned
  // reveal is surfaced. Catches wrong arg wiring or a dropped pushReveal.
  it("asks the backend with the live key and surfaces a returned reveal", async () => {
    const useStore = await listeningStoreWithKey();
    mockInvoke.mockResolvedValueOnce(sampleReveal);
    await useStore.getState().requestReveal(samplePhrase(2));
    expect(mockInvoke).toHaveBeenCalledWith("get_reveal", {
      tonic: 7,
      mode: "dorian",
      confidence: 0.9,
      phraseIndex: 2,
    });
    const q = useStore.getState().revealQueue;
    expect(q).toHaveLength(1);
    expect(q[0].reveal.connection).toBe(sampleReveal.connection);
  });

  // AC: a `null` reply is the honest "nothing to reveal" — surface nothing.
  // Catches a regression that pushes on null.
  it("honors a null reply: surfaces nothing", async () => {
    const useStore = await listeningStoreWithKey();
    mockInvoke.mockResolvedValueOnce(null);
    await useStore.getState().requestReveal(samplePhrase());
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(useStore.getState().revealQueue).toHaveLength(0);
  });

  // Best-effort: a failed request must never throw or disrupt the session.
  it("a failed reveal request never throws and leaves the queue empty", async () => {
    const useStore = await listeningStoreWithKey();
    mockInvoke.mockRejectedValueOnce(new Error("boom"));
    await expect(
      useStore.getState().requestReveal(samplePhrase()),
    ).resolves.toBeUndefined();
    expect(useStore.getState().revealQueue).toHaveLength(0);
  });
});

describe("practiceStore — coachingEnabled persistence", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("persists an explicit opt-in across fresh store instantiations", async () => {
    const useStore = await freshStore();
    // Off by default (offline-first): narration is opt-in.
    expect(useStore.getState().coachingEnabled).toBe(false);

    useStore.getState().setCoachingEnabled(true);
    expect(useStore.getState().coachingEnabled).toBe(true);
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:coaching-enabled",
      "true",
    );

    // Re-import the module to simulate a fresh tab / app restart.
    const useStore2 = await freshStore();
    expect(useStore2.getState().coachingEnabled).toBe(true);
  });

  it("persists an explicit opt-out across fresh store instantiations", async () => {
    const useStore = await freshStore();
    useStore.getState().setCoachingEnabled(false);
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:coaching-enabled",
      "false",
    );

    const useStore2 = await freshStore();
    expect(useStore2.getState().coachingEnabled).toBe(false);
  });

  it("defaults to false (off) when no preference is saved", async () => {
    const useStore = await freshStore();
    expect(useStore.getState().coachingEnabled).toBe(false);
  });
});

describe("practiceStore — practiceMode", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("defaults to 'practice' on first run", async () => {
    const useStore = await freshStore();
    expect(useStore.getState().practiceMode).toBe("practice");
  });

  it("persists the chosen mode across fresh store instantiations", async () => {
    const useStore = await freshStore();
    useStore.getState().setPracticeMode("warmup");
    expect(useStore.getState().practiceMode).toBe("warmup");
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:practice-mode",
      "warmup",
    );

    const useStore2 = await freshStore();
    expect(useStore2.getState().practiceMode).toBe("warmup");
  });

  it("falls back to 'practice' if a garbage value is persisted", async () => {
    // Force-write junk — simulates a future value we don't recognise.
    localStorageMock.setItem("ai-music-companion:practice-mode", "bogus");
    const useStore = await freshStore();
    expect(useStore.getState().practiceMode).toBe("practice");
  });

  it("startSession sends the currently-selected mode to invoke", async () => {
    const useStore = await freshStore();
    useStore.getState().setPracticeMode("run_through");

    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");

    expect(mockInvoke).toHaveBeenCalledWith("start_practice_session", {
      instrument: "Trumpet",
      practiceMode: "run_through",
      coachingEnabled: false,
      scoreId: null,
    });
  });

  it("switchInstrument sends the currently-selected mode to invoke", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");

    useStore.getState().setPracticeMode("warmup");
    mockInvoke.mockResolvedValueOnce("seg-piano");
    await useStore.getState().switchInstrument("Piano");

    expect(mockInvoke).toHaveBeenLastCalledWith("switch_instrument", {
      instrument: "Piano",
      practiceMode: "warmup",
    });
  });
});

describe("practiceStore — score loading", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  const ENTRY = {
    id: "score-1",
    title: "C Major Scale",
    composer: "Test",
    source_filename: "scale.musicxml",
    added_at: "2026-01-01T00:00:00Z",
    last_practiced_at: null,
    part_index: 0,
    duration_measures: 4,
  };
  const XML = "<score-partwise><part id='P1'/></score-partwise>";

  it("loadScoreFromId fetches MusicXML via get_score and stores it", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce({ entry: ENTRY, music_xml: XML });

    await useStore.getState().loadScoreFromId("score-1");

    // It must call the IPC that actually carries the notes — not just
    // read library metadata that lacks MusicXML.
    expect(mockInvoke).toHaveBeenCalledWith("get_score", { id: "score-1" });
    const s = useStore.getState();
    expect(s.activeScore?.id).toBe("score-1");
    expect(s.activeScoreXml).toBe(XML);
    expect(s.cursorPosition).toBeNull();
  });

  it("loadScoreFromId surfaces a useful error when get_score fails", async () => {
    const useStore = await freshStore();
    mockInvoke.mockRejectedValueOnce("score 404");

    await expect(
      useStore.getState().loadScoreFromId("missing"),
    ).rejects.toThrow(/Failed to load score/);
    // No partial state left behind.
    expect(useStore.getState().activeScoreXml).toBeNull();
  });

  it("clearActiveScore drops the loaded MusicXML and cursor", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce({ entry: ENTRY, music_xml: XML });
    await useStore.getState().loadScoreFromId("score-1");

    useStore.getState().clearActiveScore();

    const s = useStore.getState();
    expect(s.activeScore).toBeNull();
    expect(s.activeScoreXml).toBeNull();
  });
});

describe("practiceStore — follow-me accompaniment", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("startAccompaniment fires the start_accompaniment command", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce(undefined);
    await useStore.getState().startAccompaniment();
    expect(mockInvoke).toHaveBeenCalledWith("start_accompaniment");
  });

  it("startAccompaniment does not optimistically flip playing (event is authoritative)", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce(undefined);
    await useStore.getState().startAccompaniment();
    // The chip only turns on when the backend confirms via the event.
    expect(useStore.getState().accompanimentPlaying).toBe(false);
  });

  it("startAccompaniment propagates a command failure to the caller", async () => {
    const useStore = await freshStore();
    mockInvoke.mockRejectedValueOnce(new Error("no output device"));
    await expect(useStore.getState().startAccompaniment()).rejects.toThrow(
      /no output device/,
    );
  });

  it("stopAccompaniment fires the stop_accompaniment command", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce(undefined);
    await useStore.getState().stopAccompaniment();
    expect(mockInvoke).toHaveBeenCalledWith("stop_accompaniment");
  });

  it("setAccompanimentPlaying reflects the backend event", async () => {
    const useStore = await freshStore();
    useStore.getState().setAccompanimentPlaying(true);
    expect(useStore.getState().accompanimentPlaying).toBe(true);
    useStore.getState().setAccompanimentPlaying(false);
    expect(useStore.getState().accompanimentPlaying).toBe(false);
  });

  it("ending a session resets the band to not-playing", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid"); // start_practice_session
    await useStore.getState().startSession("Trumpet");
    useStore.getState().setAccompanimentPlaying(true);

    const recap: SessionRecap = {
      overall_assessment: "ok",
      strengths: [],
      areas_to_improve: [],
      next_session_suggestions: [],
      duration_secs: 10,
      phrase_count: 1,
      instrument: "Trumpet",
    };
    mockInvoke.mockResolvedValueOnce(recap);
    await useStore.getState().endSession();
    expect(useStore.getState().accompanimentPlaying).toBe(false);
  });

  it("returnToSelector clears the band playing flag", async () => {
    const useStore = await freshStore();
    useStore.getState().setAccompanimentPlaying(true);
    useStore.getState().returnToSelector();
    expect(useStore.getState().accompanimentPlaying).toBe(false);
  });

  it("setPerception stores the live snapshot and returnToSelector clears it", async () => {
    const useStore = await freshStore();
    useStore.getState().setPerception({
      tempo_bpm: 96,
      swing_ratio: null,
      locked: true,
      key: {
        tonic: 7,
        mode: "major",
        name: "G major",
        confidence: 0.6,
        alternative: { tonic: 4, minor: true, name: "E minor" },
      },
    });
    expect(useStore.getState().perception?.tempo_bpm).toBe(96);
    useStore.getState().returnToSelector();
    expect(useStore.getState().perception).toBeNull();
  });

  it("setAccompanimentKey pins the key (with a display name) and fires the command", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce(undefined);
    await useStore.getState().setAccompanimentKey(4, true);
    expect(mockInvoke).toHaveBeenCalledWith("set_accompaniment_key", {
      tonic: 4,
      minor: true,
    });
    expect(useStore.getState().keyPinned).toBe(true);
    // The pinned key carries a display name so the panel shows what's playing.
    expect(useStore.getState().pinnedKey).toEqual({
      tonic: 4,
      minor: true,
      name: "E minor",
    });
  });

  it("setAccompanimentKey rolls back the optimistic pin if the command fails", async () => {
    const useStore = await freshStore();
    mockInvoke.mockRejectedValueOnce(new Error("ipc down"));
    await useStore.getState().setAccompanimentKey(4, true);
    // The UI must not claim a pin that didn't take.
    expect(useStore.getState().keyPinned).toBe(false);
    expect(useStore.getState().pinnedKey).toBeNull();
  });

  it("lockAccompanimentKey pins the currently-perceived key; clear resumes auto", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValue(undefined);
    // Lock derives from the live perception — pin whatever is shown.
    useStore.getState().setPerception({
      tempo_bpm: 100,
      swing_ratio: null,
      locked: true,
      key: {
        tonic: 7,
        mode: "major",
        name: "G major",
        confidence: 0.7,
        alternative: { tonic: 4, minor: true, name: "E minor" },
      },
    });
    await useStore.getState().lockAccompanimentKey();
    expect(mockInvoke).toHaveBeenCalledWith("set_accompaniment_key", {
      tonic: 7,
      minor: false,
    });
    expect(useStore.getState().keyPinned).toBe(true);
    expect(useStore.getState().pinnedKey?.name).toBe("G major");

    await useStore.getState().clearAccompanimentKey();
    expect(mockInvoke).toHaveBeenCalledWith("clear_accompaniment_key");
    expect(useStore.getState().keyPinned).toBe(false);
  });

  it("ending a session resets the key pin", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");
    mockInvoke.mockResolvedValueOnce(undefined);
    await useStore.getState().setAccompanimentKey(4, true);
    expect(useStore.getState().keyPinned).toBe(true);

    const recap: SessionRecap = {
      overall_assessment: "ok",
      strengths: [],
      areas_to_improve: [],
      next_session_suggestions: [],
      duration_secs: 5,
      phrase_count: 0,
      instrument: "Trumpet",
    };
    mockInvoke.mockResolvedValueOnce(recap);
    await useStore.getState().endSession();
    expect(useStore.getState().keyPinned).toBe(false);
  });
});
