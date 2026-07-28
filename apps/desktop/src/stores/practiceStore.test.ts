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
    // Only the first START reached invoke (#453 S3's fire-and-forget
    // coaching fetch rides alongside, so filter by command).
    expect(
      mockInvoke.mock.calls.filter((c) => c[0] === "start_practice_session"),
    ).toHaveLength(1);
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

    // #419 S2b review MF6: a half-built opener — items, captured tonic,
    // direction — must not survive into the next session.
    useStore.setState({
      openerItems: [{ type: "note_sequence", degrees: [1, 2, 3] }],
      openerTonic: 9,
      openerDirection: "reversed",
      openerPreviewedDirection: "reversed",
      // #214 S1b MF5: matches and dismissals are session-scoped too.
      pieceMatch: { scoreId: "id-1", title: "Für Elise" },
      dismissedPieceIds: ["id-2"],
      // #421 S2 MF6: the click personality is session-scoped.
      pocketMode: "handoff",
      pocketFrozenBpm: 96,
      _pocketLastSentBpm: 96,
    });

    await useStore.getState().endSession();

    const s = useStore.getState();
    expect(s.status).toBe("idle");
    expect(s.screen).toBe("recap");
    expect(s.recap).toEqual(recap);
    expect(s.recapError).toBeNull();
    expect(s.openerItems).toHaveLength(0);
    expect(s.openerTonic).toBeNull();
    expect(s.openerDirection).toBe("forward");
    expect(s.openerPreviewedDirection).toBe("forward");
    expect(s.pieceMatch).toBeNull();
    expect(s.dismissedPieceIds).toHaveLength(0);
    expect(s.pocketMode).toBe("anchor");
    expect(s.pocketFrozenBpm).toBeNull();
  });

  // #341: tapping a measure MID-PRACTICE swaps to the exploration WITHOUT
  // ending the session (status stays listening, screen stays session) —
  // and a calm backend refusal (rest-only measure) shows a notice, never
  // navigation. Fails if the live path regresses to the recap handoff.
  it("exploreMeasureLive rows in place; refusals stay calm", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");
    useStore.setState({
      activeScore: { id: "score-1", title: "Etude" } as never,
    });

    const dto = { label: "C · your 4-note cell" } as never;
    mockInvoke.mockResolvedValueOnce(dto);
    await useStore.getState().exploreMeasureLive(3);
    expect(mockInvoke).toHaveBeenCalledWith("explore_measure", {
      scoreId: "score-1",
      measureNumber: 3,
    });
    let s = useStore.getState();
    expect(s.explore).toEqual(dto);
    expect(s.status).toBe("listening");
    expect(s.screen).toBe("session");

    // Calm refusal: notice set, exploration unchanged, still in session.
    useStore.setState({ explore: null, exploreNotice: null });
    mockInvoke.mockRejectedValueOnce("measure 4 is all rests — nothing to row");
    await useStore.getState().exploreMeasureLive(4);
    s = useStore.getState();
    expect(s.explore).toBeNull();
    expect(s.exploreNotice).toContain("all rests");
    expect(s.status).toBe("listening");
  });

  // #349 T4a: a jam session's endSession fetches the chord chart for the
  // recap sketch; a chart failure must not dent the recap; and a normal
  // session never even asks. Fails if the wasJam fetch or the swallow
  // breaks.
  it("endSession fetches the jam chart in room mode — best effort", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Piano");
    useStore.getState().setListenToRoom(true);

    const chart = [
      {
        label: "C",
        root_pc: 0,
        quality: "maj",
        confidence: 0.8,
        at_secs: 1.0,
        unresolved: false,
      },
    ];
    mockInvoke.mockResolvedValueOnce({
      overall_assessment: "x",
      strengths: [],
      areas_to_improve: [],
      next_session_suggestions: [],
      duration_secs: 5,
      phrase_count: 0,
      instrument: "Piano",
    });
    mockInvoke.mockResolvedValueOnce(chart);
    await useStore.getState().endSession();
    expect(mockInvoke).toHaveBeenCalledWith("session_chord_chart");
    expect(useStore.getState().jamChart).toEqual(chart);

    // startSession resets the jam state — mode is a deliberate choice.
    mockInvoke.mockResolvedValueOnce("sid2");
    await useStore.getState().startSession("Piano");
    const st = useStore.getState();
    expect(st.listenToRoom).toBe(false);
    expect(st.chordLane).toHaveLength(0);
    expect(st.jamChart).toBeNull();
  });

  it("a chart fetch failure never dents the recap", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Piano");
    useStore.getState().setListenToRoom(true);
    mockInvoke.mockResolvedValueOnce({
      overall_assessment: "x",
      strengths: [],
      areas_to_improve: [],
      next_session_suggestions: [],
      duration_secs: 5,
      phrase_count: 0,
      instrument: "Piano",
    });
    mockInvoke.mockRejectedValueOnce(new Error("chart gone"));
    await useStore.getState().endSession();
    const s = useStore.getState();
    expect(s.screen).toBe("recap");
    expect(s.recap).not.toBeNull();
    expect(s.recapError).toBeNull();
    expect(s.jamChart).toBeNull();
  });

  it("a normal session never asks for the chart", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");
    mockInvoke.mockResolvedValueOnce({
      overall_assessment: "x",
      strengths: [],
      areas_to_improve: [],
      next_session_suggestions: [],
      duration_secs: 5,
      phrase_count: 0,
      instrument: "Trumpet",
    });
    await useStore.getState().endSession();
    const calls = mockInvoke.mock.calls.map((c) => c[0]);
    expect(calls).not.toContain("session_chord_chart");
    expect(useStore.getState().jamChart).toBeNull();
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

  // #257 S4: leaving the session abandons an unfinished warmup throw — the
  // same stale-drill rule as #254 M1, and spec §6's "abandoned warmup
  // writes NOTHING". Deleting either closeWarmup() call turns these red.
  it("endSession abandons an active warmup without writing a completion", async () => {
    const useStore = await freshStore();
    // freshStore resets modules — import the SAME warmup store instance the
    // fresh practiceStore is wired to.
    const { useWarmupStore } = await import("./warmupStore");
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");
    useWarmupStore.setState({
      phase: "active",
      challenge: { seed: 5, label: "C Major", target_notes: [60] },
      playedNotes: [60, 62],
    });
    mockInvoke.mockResolvedValueOnce({
      overall_assessment: "ok",
      strengths: [],
      areas_to_improve: [],
      next_session_suggestions: [],
      duration_secs: 10,
      phrase_count: 0,
      instrument: "Trumpet",
    } satisfies SessionRecap);
    await useStore.getState().endSession();
    expect(useWarmupStore.getState().phase).toBe("idle");
    expect(
      mockInvoke.mock.calls.some((c) => c[0] === "complete_daily_warmup"),
    ).toBe(false);
  });

  it("returnToSelector abandons an active warmup without writing a completion", async () => {
    const useStore = await freshStore();
    const { useWarmupStore } = await import("./warmupStore");
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");
    useWarmupStore.setState({
      phase: "active",
      challenge: { seed: 5, label: "C Major", target_notes: [60] },
      playedNotes: [60],
    });
    useStore.getState().returnToSelector();
    expect(useWarmupStore.getState().phase).toBe("idle");
    expect(
      mockInvoke.mock.calls.some((c) => c[0] === "complete_daily_warmup"),
    ).toBe(false);
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

  // The live key/mode/confidence are passed to the backend, a returned reveal
  // is surfaced, and the unlock is persisted (#253 S3): record_reveal fires
  // with the concept+connection and the returned distinct count lands in
  // `collectionCount`. Catches wrong arg wiring, a dropped pushReveal, or a
  // dropped persistence hop.
  it("asks the backend with the live key, surfaces and records the reveal", async () => {
    const useStore = await listeningStoreWithKey();
    mockInvoke.mockResolvedValueOnce(sampleReveal); // get_reveal
    mockInvoke.mockResolvedValueOnce(3); // record_reveal → new count
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
    expect(mockInvoke).toHaveBeenCalledWith("record_reveal", {
      concept: sampleReveal.concept,
      connection: sampleReveal.connection,
    });
    expect(useStore.getState().collectionCount).toBe(3);
  });

  // A record_reveal failure is swallowed: the reveal still shows, the count
  // just stays unknown. Fails if persistence errors ever break the live loop.
  it("a failed record_reveal keeps the reveal and leaves the count unknown", async () => {
    const useStore = await listeningStoreWithKey();
    mockInvoke.mockResolvedValueOnce(sampleReveal); // get_reveal
    mockInvoke.mockRejectedValueOnce(new Error("db down")); // record_reveal
    await expect(
      useStore.getState().requestReveal(samplePhrase()),
    ).resolves.toBeUndefined();
    expect(useStore.getState().revealQueue).toHaveLength(1);
    expect(useStore.getState().collectionCount).toBeNull();
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

describe("practiceStore — guided lesson (#254)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  const drillDto = {
    index: 0,
    drill_count: 4,
    kind: "WarmupScale",
    label: "C Major",
    tempo_bpm: 60,
    difficulty: 0,
    music_xml: "<score-partwise/>",
    target_len: 8,
  };

  async function listeningStore() {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");
    await useStore.getState().startSession("Trumpet");
    mockInvoke.mockClear();
    return useStore;
  }

  // A lesson needs the mic running: refuse to start outside a live session.
  it("startLesson throws when no session is listening", async () => {
    const useStore = await freshStore();
    await expect(useStore.getState().startLesson()).rejects.toThrow();
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  // #254: starting surfaces drill 0; submitting steps to the next drill with
  // its score; the final submit lands the recap and clears the drill.
  it("start → submit → recap walks the state machine", async () => {
    const useStore = await listeningStore();
    mockInvoke.mockResolvedValueOnce({
      seed: 7,
      score: null,
      drill: drillDto,
      recap: null,
    });
    await useStore.getState().startLesson();
    expect(mockInvoke).toHaveBeenCalledWith("start_lesson", {});
    expect(useStore.getState().lessonDrill?.index).toBe(0);

    const score = {
      accuracy: 0.9,
      pitch_accuracy: 0.9,
      timing_accuracy: 0.8,
      correct: 7,
      total: 8,
    };
    mockInvoke.mockResolvedValueOnce({
      seed: 7,
      score,
      drill: { ...drillDto, index: 1 },
      recap: null,
    });
    await useStore.getState().submitDrill();
    expect(useStore.getState().lessonDrill?.index).toBe(1);
    expect(useStore.getState().lessonScore?.accuracy).toBe(0.9);

    const recap = {
      drill_labels: ["a"],
      drill_accuracies: [0.9],
      start_difficulty: 0,
      end_difficulty: 1,
    };
    mockInvoke.mockResolvedValueOnce({ seed: 7, score, drill: null, recap });
    await useStore.getState().submitDrill();
    expect(useStore.getState().lessonDrill).toBeNull();
    expect(useStore.getState().lessonRecap?.end_difficulty).toBe(1);
  });

  // A failed submit keeps the drill on screen (retryable), never throws.
  it("a failed submit keeps the current drill", async () => {
    const useStore = await listeningStore();
    mockInvoke.mockResolvedValueOnce({
      seed: 1,
      score: null,
      drill: drillDto,
      recap: null,
    });
    await useStore.getState().startLesson();
    mockInvoke.mockRejectedValueOnce(new Error("ears offline"));
    await expect(useStore.getState().submitDrill()).resolves.toBeUndefined();
    expect(useStore.getState().lessonDrill?.index).toBe(0);
  });

  // Ending abandons: state clears and the backend is told.
  it("endLesson clears state and notifies the backend", async () => {
    const useStore = await listeningStore();
    mockInvoke.mockResolvedValueOnce({
      seed: 1,
      score: null,
      drill: drillDto,
      recap: null,
    });
    await useStore.getState().startLesson();
    mockInvoke.mockResolvedValueOnce(undefined);
    await useStore.getState().endLesson();
    expect(useStore.getState().lessonDrill).toBeNull();
    expect(mockInvoke).toHaveBeenCalledWith("end_lesson", {});
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

  it("re-importing the same file keeps ONE library entry, moved to front (#385)", async () => {
    const useStore = await freshStore();
    const older = { ...ENTRY, id: "score-0", title: "Older Piece" };
    useStore.setState({ scoreLibrary: [older] });
    // The backend dedups by content and returns the SAME entry both times —
    // the store must not stack a second copy of it in the visible list.
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "import_musicxml_file") return Promise.resolve(ENTRY);
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: XML });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });

    await useStore.getState().importMusicXmlFromFile("scale.musicxml", [60], 0);
    await useStore.getState().importMusicXmlFromFile("scale.musicxml", [60], 0);

    const s = useStore.getState();
    expect(s.scoreLibrary.map((e) => e.id)).toEqual(["score-1", "score-0"]);
    expect(s.activeScore?.id).toBe("score-1");
  });
});

describe("practiceStore — follow-me accompaniment", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("startAccompaniment carries the Pocket's set tempo (#445 pt 9)", async () => {
    const useStore = await freshStore();
    useStore.getState().setPocketTempo(104);
    mockInvoke.mockResolvedValueOnce(undefined);
    await useStore.getState().startAccompaniment();
    // The band replaces the click, so it must BE the clock — it starts at
    // the exact tempo the click would have played.
    expect(mockInvoke).toHaveBeenCalledWith("start_accompaniment", {
      tempoBpm: 104,
    });
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

describe("practiceStore — the band carries the Pocket clock (#445 pt 9)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  // The same shape PocketControl.test.tsx uses for the click's gates.
  const perceptionWithTempo = (bpm: number | null, locked = bpm !== null) =>
    ({
      tempo_bpm: bpm,
      swing_ratio: null,
      locked,
      key: null,
      chord: null,
      hearing_polyphony: false,
    }) as never;

  it("follow mode streams set_band_tempo under the click's exact gates", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(100_000);
    const useStore = await freshStore();
    useStore.setState({
      accompanimentPlaying: true,
      pocketPlaying: false,
      pocketMode: "follow",
      _pocketFollowStartedAt: 100_000,
    });
    const send = (bpm: number | null, locked?: boolean) =>
      useStore.getState().setPerception(perceptionWithTempo(bpm, locked));
    send(96);
    expect(mockInvoke).toHaveBeenCalledWith("set_band_tempo", {
      tempoBpm: 96,
    });
    // Within the throttle window: nothing, even on a big change.
    send(120);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    // Past the throttle but under the 2-BPM delta: nothing.
    vi.setSystemTime(101_500);
    send(97);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    // Past both gates: sends.
    send(104);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
    // Absent, out-of-range, or UNLOCKED readings: never chased.
    vi.setSystemTime(103_000);
    send(null);
    send(500);
    send(150, false);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
    // One clock, one carrier: the band's stream never touches the click's
    // command.
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_pocket_tempo",
      expect.anything(),
    );
    vi.useRealTimers();
  });

  it("anchor mode streams nothing to the band", async () => {
    const useStore = await freshStore();
    useStore.setState({
      accompanimentPlaying: true,
      pocketPlaying: false,
      pocketMode: "anchor",
    });
    useStore.getState().setPerception(perceptionWithTempo(96));
    // Anchor = the set BPM installed at band start; no retime stream.
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("handoff follows the band, then freezes and the stream stops", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(200_000);
    const useStore = await freshStore();
    useStore.setState({ accompanimentPlaying: true, pocketPlaying: false });
    useStore.getState().setPocketMode("handoff"); // window anchors here
    const send = (bpm: number) =>
      useStore.getState().setPerception(perceptionWithTempo(bpm));
    send(96); // follows during the window
    expect(mockInvoke).toHaveBeenCalledWith("set_band_tempo", {
      tempoBpm: 96,
    });
    // The window closes: the next reading freezes instead of sending.
    vi.setSystemTime(209_000);
    send(98);
    expect(useStore.getState().pocketFrozenBpm).toBe(96);
    // Frozen: no further set_band_tempo sends.
    send(101);
    expect(
      mockInvoke.mock.calls.filter(([c]) => c === "set_band_tempo").length,
    ).toBe(1);
    vi.useRealTimers();
  });

  it("a fresh band start begins a fresh follow life — no stale freeze or delta", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(400_000);
    const useStore = await freshStore();
    // Stale state from an earlier carrier's handoff life.
    useStore.setState({
      accompanimentPlaying: false,
      pocketMode: "handoff",
      pocketFrozenBpm: 96,
      _pocketLastSentBpm: 96,
      _pocketFollowStartedAt: 0,
    });
    useStore.getState().setAccompanimentPlaying(true);
    expect(useStore.getState().pocketFrozenBpm).toBeNull();
    expect(useStore.getState()._pocketLastSentBpm).toBeNull();
    // The follow window anchors at the band's real start: within it,
    // handoff FOLLOWS (stale anchor would have frozen instantly).
    useStore.getState().setPerception(perceptionWithTempo(97));
    expect(mockInvoke).toHaveBeenCalledWith("set_band_tempo", {
      tempoBpm: 97,
    });
    vi.useRealTimers();
  });

  it("room mode starts the band with no override and streams nothing to it", async () => {
    // #445 pt 9 review MF2: in "listen to the room" the room's live
    // players ARE the clock — the band must keep the legacy
    // listen-and-join path (null override) and the follow policy must
    // never stream set_band_tempo at it.
    vi.useFakeTimers();
    vi.setSystemTime(600_000);
    const useStore = await freshStore();
    mockInvoke.mockResolvedValue(undefined);
    useStore.setState({ listenToRoom: true });
    await useStore.getState().startAccompaniment();
    expect(mockInvoke).toHaveBeenCalledWith("start_accompaniment", {
      tempoBpm: null,
    });
    useStore.setState({
      accompanimentPlaying: true,
      pocketPlaying: false,
      pocketMode: "follow",
      _pocketFollowStartedAt: 600_000,
    });
    useStore.getState().setPerception(perceptionWithTempo(96));
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_band_tempo",
      expect.anything(),
    );
    vi.useRealTimers();
  });

  it("the pocket outranks the band if state ever claims both are playing", async () => {
    // Backend-side they are mutually exclusive; if frontend state ever
    // desyncs, the click (the audible metronome) owns the stream.
    vi.useFakeTimers();
    vi.setSystemTime(500_000);
    const useStore = await freshStore();
    useStore.setState({
      pocketPlaying: true,
      accompanimentPlaying: true,
      pocketMode: "follow",
      _pocketFollowStartedAt: 500_000,
    });
    useStore.getState().setPerception(perceptionWithTempo(96));
    expect(mockInvoke).toHaveBeenCalledWith("set_pocket_tempo", {
      tempoBpm: 96,
    });
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_band_tempo",
      expect.anything(),
    );
    vi.useRealTimers();
  });
});

describe("practiceStore — the coaching box (#453 S3)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  const trend = {
    kind: "trend",
    text: "Your Eb major rows are sitting at 54% over 6 attempts (last one 2 days ago) — worth a slow pass.",
    evidence:
      "key_mastery 3:major: 6 attempts, accuracy EWMA 0.54, last attempt 2d ago",
  };
  const momentum = {
    kind: "momentum",
    text: "Your 3-note cell (0 4 7) climbed from 50% to 80% across its last 8 graded rows — push the tempo.",
    evidence: "cell [0 4 7]: older-half mean 0.50 → newer-half mean 0.80",
  };

  /** Let the fire-and-forget refresh settle. */
  const flush = () => new Promise((r) => setTimeout(r, 0));

  // AC9: session start fires the history fetch, routed by command name,
  // and the FIRST pinned suggestion lands in the store. Fails if the
  // startSession hook is dropped or the store keeps more than one.
  it("session start fetches a coaching suggestion (#453 S3)", async () => {
    const useStore = await freshStore();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "start_practice_session") return Promise.resolve("sid");
      if (cmd === "practice_suggestions")
        return Promise.resolve([trend, momentum]);
      return Promise.resolve(null);
    });
    await useStore.getState().startSession("Trumpet");
    await flush();
    expect(mockInvoke.mock.calls.map((c) => c[0])).toContain(
      "practice_suggestions",
    );
    expect(useStore.getState().coachingSuggestion).toEqual(trend);
  });

  // AC9 + rule 0: explore begin refreshes; an EMPTY analyzer result never
  // clears the shown suggestion; session end resets the box (suggestion
  // cleared, quiet lifted). Fails if empties clear, explore stops
  // refreshing, or the box leaks across sessions.
  it("explore begin refreshes; empty results never clear; session end resets", async () => {
    const useStore = await freshStore();
    let suggestions: unknown[] = [trend];
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "start_practice_session") return Promise.resolve("sid");
      if (cmd === "practice_suggestions") return Promise.resolve(suggestions);
      if (cmd === "start_explore_variation")
        return Promise.resolve({
          label: "x",
          music_xml: "<score-partwise/>",
          chips: [],
          root_pitch_classes: [0],
        });
      if (cmd === "end_practice_session") return Promise.resolve({});
      return Promise.resolve(null);
    });
    await useStore.getState().startSession("Trumpet");
    await flush();
    expect(useStore.getState().coachingSuggestion).toEqual(trend);

    // Explore begin refreshes — the analyzer now says NOTHING, and the
    // box must hold what it has (rule 0).
    suggestions = [];
    await useStore.getState().startExplore(0, "major");
    await flush();
    expect(
      mockInvoke.mock.calls.filter((c) => c[0] === "practice_suggestions"),
    ).toHaveLength(2);
    expect(useStore.getState().coachingSuggestion).toEqual(trend);

    // A NEWER suggestion replaces in place through the same hook.
    suggestions = [momentum];
    await useStore.getState().startExplore(0, "major");
    await flush();
    expect(useStore.getState().coachingSuggestion).toEqual(momentum);

    // Session end resets the box AND the quiet for the next session.
    useStore.setState({ coachingQuieted: true });
    await useStore.getState().endSession();
    expect(useStore.getState().coachingSuggestion).toBeNull();
    expect(useStore.getState().coachingQuieted).toBe(false);
  });

  // Dismissal quiets for the session: later fetches must NOT resurface
  // a suggestion. Fails if refresh ignores the quiet flag.
  it("dismissal quiets the box against later fetches", async () => {
    const useStore = await freshStore();
    useStore.setState({ coachingSuggestion: trend });
    useStore.getState().dismissCoachingSuggestion();
    expect(useStore.getState().coachingSuggestion).toBeNull();
    mockInvoke.mockResolvedValueOnce([momentum]);
    await useStore.getState().refreshCoachingSuggestion();
    expect(useStore.getState().coachingSuggestion).toBeNull();
  });

  // Review R1: a fetch that resolves AFTER its session ended must not
  // write a stale suggestion into the next session (the seq-token
  // invalidation — same discipline as _openerRefreshSeq). Fails if the
  // post-await seq check is dropped.
  it("a fetch resolving after a session boundary writes nothing", async () => {
    const useStore = await freshStore();
    let resolveFetch: (v: unknown) => void = () => {};
    mockInvoke.mockReturnValueOnce(
      new Promise((res) => {
        resolveFetch = res;
      }),
    );
    const inflight = useStore.getState().refreshCoachingSuggestion();
    // The session boundary bumps the token (endSession-tail style).
    useStore.setState((s) => ({
      _coachingFetchSeq: s._coachingFetchSeq + 1,
    }));
    resolveFetch([trend]);
    await inflight;
    expect(useStore.getState().coachingSuggestion).toBeNull();
  });

  // Fetch failure is silent — the box neither crashes nor clears.
  it("a failed fetch never throws and never clears", async () => {
    const useStore = await freshStore();
    useStore.setState({ coachingSuggestion: trend });
    mockInvoke.mockRejectedValueOnce(new Error("boom"));
    await useStore.getState().refreshCoachingSuggestion();
    expect(useStore.getState().coachingSuggestion).toEqual(trend);
  });
});

describe("practiceStore — the method-book tip voice (#454 S3)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  const trend = {
    kind: "trend",
    text: "Your Eb major rows are sitting at 54% over 6 attempts (last one 2 days ago) — worth a slow pass.",
    evidence:
      "key_mastery 3:major: 6 attempts, accuracy EWMA 0.54, last attempt 2d ago",
  };
  const schlossberg = {
    topic: "Long tones and pitch stability",
    guidance:
      "There are drills for exactly this in Schlossberg's Daily Drills — start the note softly, let it grow, and keep the pitch absolutely level.",
    source_line: "Max Schlossberg, Daily Drills and Technical Studies",
  };

  /** Let the fire-and-forget refresh settle. */
  const flush = () => new Promise((r) => setTimeout(r, 0));

  // #454 AC8: the SAME refresh point (session start) fetches the tip
  // alongside history, routed by command name; session end resets the tip
  // with the rest of the box. Fails if the tip gets its own diverging
  // refresh cadence or leaks across sessions.
  it("session start fetches the method-book tip alongside history (#454 S3)", async () => {
    const useStore = await freshStore();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "start_practice_session") return Promise.resolve("sid");
      if (cmd === "practice_suggestions") return Promise.resolve([]);
      if (cmd === "method_book_tip") return Promise.resolve(schlossberg);
      if (cmd === "end_practice_session") return Promise.resolve({});
      return Promise.resolve(null);
    });
    await useStore.getState().startSession("Trumpet");
    await flush();
    const calls = mockInvoke.mock.calls.map((c) => c[0]);
    expect(calls).toContain("practice_suggestions");
    expect(calls).toContain("method_book_tip");
    expect(useStore.getState().coachingTip).toEqual(schlossberg);
    // History said nothing — the history voice stays empty (no filler).
    expect(useStore.getState().coachingSuggestion).toBeNull();

    // Session end resets the tip voice too.
    await useStore.getState().endSession();
    expect(useStore.getState().coachingTip).toBeNull();
    expect(useStore.getState().coachingQuieted).toBe(false);
  });

  // #454 AC7: rule 0 for the tip voice — an empty tip result never clears
  // a shown tip — and ONE dismissal quiets BOTH voices against later
  // fetches carrying fresh material. Fails if empties clear the tip or a
  // dismissal only silences one voice.
  it("the tip voice holds through empty fetches and dismissal quiets both", async () => {
    const useStore = await freshStore();
    let tip: unknown = schlossberg;
    let suggestions: unknown[] = [];
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "practice_suggestions") return Promise.resolve(suggestions);
      if (cmd === "method_book_tip") return Promise.resolve(tip);
      return Promise.resolve(null);
    });
    await useStore.getState().refreshCoachingSuggestion();
    expect(useStore.getState().coachingTip).toEqual(schlossberg);

    // Rule 0: the engine's calm None never clears the shown tip.
    tip = null;
    await useStore.getState().refreshCoachingSuggestion();
    expect(useStore.getState().coachingTip).toEqual(schlossberg);

    // One dismissal quiets BOTH voices…
    useStore.getState().dismissCoachingSuggestion();
    expect(useStore.getState().coachingTip).toBeNull();
    expect(useStore.getState().coachingSuggestion).toBeNull();
    // …including against later fetches with fresh material in both.
    tip = schlossberg;
    suggestions = [trend];
    await useStore.getState().refreshCoachingSuggestion();
    expect(useStore.getState().coachingTip).toBeNull();
    expect(useStore.getState().coachingSuggestion).toBeNull();
  });

  // #454 AC7: the seq token guards the tip voice too — a tip resolving
  // after a session boundary writes nothing into the next session. Fails
  // if the post-await seq check stops covering the tip.
  // Review note 1: a malformed tip payload (mis-mocked backend, foreign
  // shape) must be rejected by the runtime shape guard — tsc can't see
  // wire payloads. Fails if the guard is dropped.
  it("a malformed tip payload never enters the box", async () => {
    const useStore = await freshStore();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "practice_suggestions") return Promise.resolve([]);
      if (cmd === "method_book_tip") return Promise.resolve({ topic: 1 });
      return Promise.resolve(null);
    });
    await useStore.getState().refreshCoachingSuggestion();
    expect(useStore.getState().coachingTip).toBeNull();
  });

  // Review note 2: the two voices fetch independently — a history
  // rejection must not poison a fulfilled tip. Fails if allSettled
  // regresses to Promise.all with a shared catch.
  it("the tip still applies when the history fetch rejects", async () => {
    const useStore = await freshStore();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "practice_suggestions")
        return Promise.reject(new Error("boom"));
      if (cmd === "method_book_tip") return Promise.resolve(schlossberg);
      return Promise.resolve(null);
    });
    await useStore.getState().refreshCoachingSuggestion();
    expect(useStore.getState().coachingTip).toEqual(schlossberg);
    expect(useStore.getState().coachingSuggestion).toBeNull();
  });

  it("a tip resolving after a session boundary writes nothing", async () => {
    const useStore = await freshStore();
    let resolveTip: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "practice_suggestions") return Promise.resolve([]);
      if (cmd === "method_book_tip")
        return new Promise((res) => {
          resolveTip = res;
        });
      return Promise.resolve(null);
    });
    const inflight = useStore.getState().refreshCoachingSuggestion();
    // The session boundary bumps the token (endSession-tail style).
    useStore.setState((s) => ({
      _coachingFetchSeq: s._coachingFetchSeq + 1,
    }));
    resolveTip(schlossberg);
    await inflight;
    expect(useStore.getState().coachingTip).toBeNull();
  });
});
