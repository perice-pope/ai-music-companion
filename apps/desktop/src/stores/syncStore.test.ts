import { describe, it, expect, beforeEach, vi } from "vitest";
import type {
  SessionSummaryDto,
  StoredSessionDto,
  TasteProfile,
} from "../types/brain";

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

function tasteProfile(): TasteProfile {
  return {
    genres: ["hip-hop", "gospel"],
    artists: ["Kendrick Lamar"],
    goals: ["audition prep"],
    experience: "intermediate",
    is_under_13: false,
  };
}

/** Wire invoke to return summaries then per-id details (+ taste profile). */
/** The blob wireInvoke serves for get_learner_model_blob (tests may reassign). */
let learnerBlob: unknown = { version: 1, collection: {}, difficulty: 2 };

function wireInvoke(summaries: SessionSummaryDto[]) {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === "get_session_history") return summaries;
    if (cmd === "get_session_detail") {
      const { session_id } = args as { session_id: string };
      return detail(session_id);
    }
    if (cmd === "get_taste_profile") return tasteProfile();
    if (cmd === "get_learner_model_blob") return learnerBlob;
    throw new Error(`unexpected command ${cmd}`);
  });
}

describe("syncStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    mockUpsert.mockResolvedValue({ error: null });
    learnerBlob = { version: 1, collection: {}, difficulty: 2 };
  });

  it("is a no-op when called without a user id", async () => {
    const useStore = await freshStore();
    await useStore.getState().syncAll(null, true);
    expect(useStore.getState().status).toBe("idle");
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(mockUpsert).not.toHaveBeenCalled();
  });

  it("is a no-op when not opted in (switch off)", async () => {
    const useStore = await freshStore();
    await useStore.getState().syncAll("user-1", false);
    expect(useStore.getState().status).toBe("idle");
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(mockUpsert).not.toHaveBeenCalled();
  });

  it("pushes unsynced sessions and maps the recap onto the row", async () => {
    wireInvoke([summary("s1"), summary("s2")]);
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1", true);

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
    // The full fingerprint is pushed alongside the legacy tone projection.
    expect(rows[0].fingerprint).toMatchObject({
      tone: { brightness: 0.6, warmth: 0.5 },
    });

    const s = useStore.getState();
    expect(s.status).toBe("synced");
    expect(s.syncedThisRun).toBe(2);
    expect(s.lastSyncedAt).not.toBeNull();
  });

  it("skips sessions already synced for that user on a second run", async () => {
    wireInvoke([summary("s1")]);
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1", true);
    expect(useStore.getState().syncedThisRun).toBe(1);

    // Second run: s1 is remembered, nothing new to push.
    mockUpsert.mockClear();
    await useStore.getState().syncAll("user-1", true);
    expect(mockUpsert).not.toHaveBeenCalled();
    expect(useStore.getState().status).toBe("synced");
    expect(useStore.getState().syncedThisRun).toBe(0);
  });

  it("tracks synced ids per user (a different user re-pushes)", async () => {
    wireInvoke([summary("s1")]);
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1", true);
    mockUpsert.mockClear();

    await useStore.getState().syncAll("user-2", true);
    expect(mockUpsert).toHaveBeenCalledTimes(1);
    expect(useStore.getState().syncedThisRun).toBe(1);
  });

  it("surfaces an upsert failure as error status without marking synced", async () => {
    wireInvoke([summary("s1")]);
    mockUpsert.mockResolvedValue({ error: { message: "rls denied" } });
    const useStore = await freshStore();

    await useStore.getState().syncAll("user-1", true);
    expect(useStore.getState().status).toBe("error");
    expect(useStore.getState().error).toContain("rls denied");

    // A retry after the failure should attempt the push again (not skipped).
    mockUpsert.mockResolvedValue({ error: null });
    await useStore.getState().syncAll("user-1", true);
    expect(useStore.getState().status).toBe("synced");
    expect(useStore.getState().syncedThisRun).toBe(1);
  });
});

describe("syncStore.syncTasteProfile (independent opt-in)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    mockUpsert.mockResolvedValue({ error: null });
  });

  it("is a no-op when not opted in (switch off)", async () => {
    wireInvoke([]);
    const useStore = await freshStore();

    await useStore.getState().syncTasteProfile("user-1", false);

    expect(mockInvoke).not.toHaveBeenCalled();
    expect(mockUpsert).not.toHaveBeenCalled();
    expect(useStore.getState().tasteProfileStatus).toBe("idle");
  });

  it("is a no-op without a user id even when opted in", async () => {
    wireInvoke([]);
    const useStore = await freshStore();

    await useStore.getState().syncTasteProfile(null, true);

    expect(mockUpsert).not.toHaveBeenCalled();
    expect(useStore.getState().tasteProfileStatus).toBe("idle");
  });

  it("pushes the profile to its own table when opted in", async () => {
    wireInvoke([]);
    const useStore = await freshStore();

    await useStore.getState().syncTasteProfile("user-1", true);

    expect(mockInvoke).toHaveBeenCalledWith("get_taste_profile");
    expect(mockFrom).toHaveBeenCalledWith("taste_profile");
    // Two upserts now: the taste row, then the chained learner-model push
    // (progress data rides the same opt-in — see syncLearnerModel tests).
    await new Promise((r) => setTimeout(r, 0));
    expect(mockUpsert).toHaveBeenCalledTimes(2);
    const [row, opts] = mockUpsert.mock.calls[0];
    expect(opts).toEqual({ onConflict: "user_id" });
    expect(row).toMatchObject({
      user_id: "user-1",
      genres: ["hip-hop", "gospel"],
      artists: ["Kendrick Lamar"],
      goals: ["audition prep"],
      experience: "intermediate",
      is_under_13: false,
    });
    expect(useStore.getState().tasteProfileStatus).toBe("synced");
  });

  it("does not touch the sessions table (decoupled from session sync)", async () => {
    wireInvoke([]);
    const useStore = await freshStore();

    await useStore.getState().syncTasteProfile("user-1", true);

    expect(mockFrom).not.toHaveBeenCalledWith("sessions");
    // Session sync state is untouched by a profile sync.
    expect(useStore.getState().status).toBe("idle");
  });

  it("surfaces an upsert failure on the profile-specific status", async () => {
    wireInvoke([]);
    mockUpsert.mockResolvedValue({ error: { message: "rls denied" } });
    const useStore = await freshStore();

    await useStore.getState().syncTasteProfile("user-1", true);

    expect(useStore.getState().tasteProfileStatus).toBe("error");
    expect(useStore.getState().tasteProfileError).toContain("rls denied");
    // The session-sync status is independent and stays clean.
    expect(useStore.getState().status).toBe("idle");
  });
});
describe("syncLearnerModel (#252 F2)", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));

  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    mockUpsert.mockResolvedValue({ error: null });
    learnerBlob = { version: 1, collection: {}, difficulty: 2 };
  });

  // The blob is pushed to learner_model keyed on the user — gutting the
  // function (or dropping the chained call from taste sync) fails here.
  it("pushes the local blob keyed on the user", async () => {
    wireInvoke([]);
    const useStore = await freshStore();
    await useStore.getState().syncLearnerModel("user-1", true);
    expect(mockFrom).toHaveBeenCalledWith("learner_model");
    const [row, opts] = mockUpsert.mock.calls.at(-1)!;
    expect(row.user_id).toBe("user-1");
    expect(row.model).toEqual({ version: 1, collection: {}, difficulty: 2 });
    expect(opts).toEqual({ onConflict: "user_id" });
  });

  it("does nothing when signed out, opted out, or on a cold start", async () => {
    wireInvoke([]);
    const useStore = await freshStore();
    await useStore.getState().syncLearnerModel(null, true);
    await useStore.getState().syncLearnerModel("user-1", false);
    learnerBlob = null; // cold start — nothing to push
    await useStore.getState().syncLearnerModel("user-1", true);
    expect(mockUpsert).not.toHaveBeenCalled();
  });

  // Best-effort: a learner push failure must not disturb the taste status.
  it("a failed learner push leaves taste sync status alone", async () => {
    wireInvoke([]);
    mockUpsert
      .mockResolvedValueOnce({ error: null }) // taste upsert
      .mockResolvedValueOnce({ error: { message: "boom" } }); // learner upsert
    const useStore = await freshStore();
    await useStore.getState().syncTasteProfile("user-1", true);
    await flush();
    expect(useStore.getState().tasteProfileStatus).toBe("synced");
  });

  // Taste sync chains the learner push under the same opt-in.
  it("taste sync also pushes the learner model", async () => {
    wireInvoke([]);
    const useStore = await freshStore();
    await useStore.getState().syncTasteProfile("user-1", true);
    await flush();
    const tables = mockFrom.mock.calls.map((c) => c[0]);
    expect(tables).toContain("taste_profile");
    expect(tables).toContain("learner_model");
  });
});
