import { describe, it, expect, beforeEach, vi } from "vitest";

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

// The slice is exercised through the composed store: these tests pin both
// the click's behavior AND that the #511 composition actually wires the
// slice's state and actions onto usePracticeStore.
async function freshStore() {
  vi.resetModules();
  const mod = await import("../practiceStore");
  return mod.usePracticeStore;
}

describe("pocketSlice — tempo persistence (#421 S1)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("first run: anchor personality, count-in on, 90 BPM, not playing", async () => {
    // The #421 S1 contract's defaults — a default-follow click would chase
    // the player's pulse on first open; a default-off count-in breaks the
    // "1-2-3-4!" call-off promise. Pinned because only initial values reach
    // users unset (every other test setStates past them).
    const useStore = await freshStore();
    const s = useStore.getState();
    expect(s.pocketMode).toBe("anchor");
    expect(s.pocketCountIn).toBe(true);
    expect(s.pocketTempo).toBe(90);
    expect(s.pocketPlaying).toBe(false);
  });

  it("persists a set tempo across fresh store instantiations", async () => {
    const useStore = await freshStore();
    expect(useStore.getState().pocketTempo).toBe(90); // spec §4 default

    useStore.getState().setPocketTempo(104);
    expect(useStore.getState().pocketTempo).toBe(104);
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:pocket-tempo",
      "104",
    );

    // Re-import the module to simulate a fresh tab / app restart.
    const useStore2 = await freshStore();
    expect(useStore2.getState().pocketTempo).toBe(104);
  });

  it.each(["fast", "0", "-3", "NaN"])(
    "a corrupt stored tempo (%s) falls back to the 90 default",
    async (raw) => {
      store["ai-music-companion:pocket-tempo"] = raw;
      const useStore = await freshStore();
      expect(useStore.getState().pocketTempo).toBe(90);
    },
  );
});

describe("pocketSlice — backend-authoritative status (#421)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
  });

  it("startPocket sends the store's semantic settings, nothing clamped", async () => {
    const useStore = await freshStore();
    mockInvoke.mockResolvedValueOnce(undefined);
    useStore.getState().setPocketTempo(300); // clamping is the backend's job
    useStore.getState().setPocketCountIn(false);

    await useStore.getState().startPocket();

    expect(mockInvoke).toHaveBeenCalledWith("start_pocket", {
      tempoBpm: 300,
      beatsPerBar: 4,
      countIn: false,
    });
  });

  it("mirrors the backend's clamped tempo while playing, but never a 0", async () => {
    const useStore = await freshStore();
    useStore.getState().setPocketTempo(104);

    useStore.getState().setPocketStatus(true, 112);
    expect(useStore.getState().pocketPlaying).toBe(true);
    expect(useStore.getState().pocketTempo).toBe(112);

    // A zero/absent tempo report must not wipe the real setting.
    useStore.getState().setPocketStatus(true, 0);
    expect(useStore.getState().pocketTempo).toBe(112);

    // Stop reports don't touch the set tempo either.
    useStore.getState().setPocketStatus(false, 250);
    expect(useStore.getState().pocketPlaying).toBe(false);
    expect(useStore.getState().pocketTempo).toBe(112);
  });

  it("a FRESH click resets the follow life; a repeat status event does not", async () => {
    const useStore = await freshStore();
    // Fake ONLY Date so the two events sit at provably different clock
    // readings — with the real clock they can share a millisecond and an
    // always-re-anchor mutation would pass unseen.
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(5_000);
      // Stale follow state from a previous click (review MF4).
      useStore.setState({
        pocketPlaying: false,
        pocketMode: "handoff",
        pocketFrozenBpm: 96,
        _pocketLastSentBpm: 96,
        _pocketFollowStartedAt: 1,
      });

      useStore.getState().setPocketStatus(true, 100);
      expect(useStore.getState().pocketFrozenBpm).toBeNull();
      expect(useStore.getState()._pocketLastSentBpm).toBeNull();
      expect(useStore.getState()._pocketFollowStartedAt).toBe(5_000);

      // Mid-play the handoff window freezes a tempo; a REPEAT playing:true
      // event (backend re-emit) must not melt it or re-anchor the window.
      vi.setSystemTime(6_000);
      useStore.setState({ pocketFrozenBpm: 98, _pocketLastSentBpm: 98 });
      useStore.getState().setPocketStatus(true, 100);
      expect(useStore.getState().pocketFrozenBpm).toBe(98);
      expect(useStore.getState()._pocketLastSentBpm).toBe(98);
      expect(useStore.getState()._pocketFollowStartedAt).toBe(5_000);
    } finally {
      vi.useRealTimers();
    }
  });
});
