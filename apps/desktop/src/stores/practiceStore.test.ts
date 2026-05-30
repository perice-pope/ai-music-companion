import { describe, it, expect, beforeEach, vi } from "vitest";
import type { CoachingTip, SessionRecap } from "../types/brain";

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
    // invoke called with the right command + args
    expect(mockInvoke).toHaveBeenCalledWith("start_practice_session", {
      instrument: "Trumpet",
      practiceMode: "practice",
      coachingEnabled: true,
      scoreId: null,
    });
  });

  it("startSession is rejected when already listening", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce("sid");

    await useStore.getState().startSession("Trumpet");
    expect(useStore.getState().status).toBe("listening");

    // Second start must throw — and must NOT call invoke a second time.
    await expect(
      useStore.getState().startSession("Piano"),
    ).rejects.toThrow(/cannot start session from status=listening/);
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
    await expect(
      useStore.getState().switchInstrument("Piano"),
    ).rejects.toThrow(/cannot switch instrument from status=idle/);
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

describe("practiceStore — coachingEnabled persistence", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("persists false across fresh store instantiations", async () => {
    const useStore = await freshStore();
    expect(useStore.getState().coachingEnabled).toBe(true);

    useStore.getState().setCoachingEnabled(false);
    expect(useStore.getState().coachingEnabled).toBe(false);
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:coaching-enabled",
      "false",
    );

    // Re-import the module to simulate a fresh tab / app restart.
    const useStore2 = await freshStore();
    expect(useStore2.getState().coachingEnabled).toBe(false);
  });

  it("defaults to true when no preference is saved", async () => {
    const useStore = await freshStore();
    expect(useStore.getState().coachingEnabled).toBe(true);
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
      coachingEnabled: true,
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
