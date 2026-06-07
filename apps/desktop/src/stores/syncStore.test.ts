import { describe, it, expect, beforeEach, vi } from "vitest";
import type { SessionSummaryDto, StoredSessionDto } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Controllable Supabase upsert mock.
const mockUpsert = vi.fn();
const mockFrom = vi.fn((..._args: unknown[]) => ({ upsert: mockUpsert }));
vi.mock("../lib/supabase", () => ({
  supabase: { from: (...args: unknown[]) => mockFrom(...args) },
}));

// localStorage polyfill with real tracking (jsdom in CI doesn't persist).
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
  key: vi.fn(() => null),
};
Object.defineProperty(window, "localStorage", { value: localStorageMock });

async function freshStore() {
  vi.resetModules();
  const mod = await import("./syncStore");
  return mod.useSyncStore;
}

function summary(id: string): SessionSummaryDto {
  return {
    id,
    instrument: "Trumpet",
    started_at: "2026-05-31T10:00:00Z",
    duration_secs: 600,
    phrase_count: 12,
  };
}

function detail(id: string): StoredSessionDto {
  return {
    id,
    started_at: "2026-05-31T10:00:00Z",
    ended_at: "2026-05-31T10:10:00Z",
    recap: {
      overall_assessment: "Solid work.",
      strengths: ["steady tone"],
      areas_to_improve: ["attacks"],
      next_session_suggestions: ["long tones"],
      duration_secs: 600,
      phrase_count: 12,
      instrument: "Trumpet",
      fingerprint: {
        tone: {
          brightness: 0.6,
          warmth: 0.5,
          air_noise: 0.2,
          core_clarity: 0.7,
          vibrato_quality: 0.4,
        },
      },
    },
  };
}

/** Wire invoke to return summaries then per-id details. */
function wireInvoke(summaries: SessionSummaryDto[]) {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === "get_session_history") return summaries;
    if (cmd === "get_session_detail") {
      const { session_id } = args as { session_id: string };
      return detail(session_id);
    }
    throw new Error(`unexpected command ${cmd}`);
  });
}

describe("syncStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    mockUpsert.mockResolvedValue({ error: null });
  });

  it("is a no-op when called without a user id", async () => {
    const useStore = await freshStore();
    await useStore.getState().syncAll(null);
    expect(useStore.getState().status).toBe("idle");
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(mockUpsert).not.toHaveBeenCalled();
  });

  it("pushes unsynced sessions and maps the recap onto the row", async () => {
    wireInvoke([summary("s1"), summary("s2")]);
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1");

    expect(mockFrom).toHaveBeenCalledWith("sessions");
    expect(mockUpsert).toHaveBeenCalledTimes(1);
    const [rows, opts] = mockUpsert.mock.calls[0];
    expect(opts).toEqual({ onConflict: "id" });
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      id: "s1",
      student_id: "user-1",
      instrument: "Trumpet",
      duration_secs: 600,
      phrase_count: 12,
      overall_assessment: "Solid work.",
    });
    expect(rows[0].session_tone).toMatchObject({ brightness: 0.6 });

    const s = useStore.getState();
    expect(s.status).toBe("synced");
    expect(s.syncedThisRun).toBe(2);
    expect(s.lastSyncedAt).not.toBeNull();
  });

  it("skips sessions already synced for that user on a second run", async () => {
    wireInvoke([summary("s1")]);
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1");
    expect(useStore.getState().syncedThisRun).toBe(1);

    // Second run: s1 is remembered, nothing new to push.
    mockUpsert.mockClear();
    await useStore.getState().syncAll("user-1");
    expect(mockUpsert).not.toHaveBeenCalled();
    expect(useStore.getState().status).toBe("synced");
    expect(useStore.getState().syncedThisRun).toBe(0);
  });

  it("tracks synced ids per user (a different user re-pushes)", async () => {
    wireInvoke([summary("s1")]);
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1");
    mockUpsert.mockClear();

    await useStore.getState().syncAll("user-2");
    expect(mockUpsert).toHaveBeenCalledTimes(1);
    expect(useStore.getState().syncedThisRun).toBe(1);
  });

  it("surfaces an upsert failure as error status without marking synced", async () => {
    wireInvoke([summary("s1")]);
    mockUpsert.mockResolvedValue({ error: { message: "rls denied" } });
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1");
    expect(useStore.getState().status).toBe("error");
    expect(useStore.getState().error).toContain("rls denied");

    // A retry after the failure should attempt the push again (not skipped).
    mockUpsert.mockResolvedValue({ error: null });
    await useStore.getState().syncAll("user-1");
    expect(useStore.getState().status).toBe("synced");
    expect(useStore.getState().syncedThisRun).toBe(1);
  });
});
