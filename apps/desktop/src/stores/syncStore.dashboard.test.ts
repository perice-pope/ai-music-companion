import { describe, it, expect, beforeEach, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type {
  ExerciseFactRow,
  SessionProjectionDto,
  SessionSummaryDto,
} from "../types/brain";

/**
 * #449 T2: the teacher-dashboard projection (spec docs/specs/449-t2-sync-projection.md).
 * Covers the gating matrix (AC1), the column-for-column pushes (AC2/AC3),
 * the structural spec_json/seed absence (AC4), idempotency (AC5), the
 * exercise watermark (AC6), calm failure (AC7), and the #470 option-(b)
 * narration-semantics note at the projection site (AC9).
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// ── A chainable Supabase mock covering the projection's query shapes ──────
// upsert(...).select(...).single(), bare awaited upsert(...), insert(...).
// select(...).single(), select(...).eq().eq().limit().maybeSingle(), and
// select(...).in(). Every call is recorded for assertions.
interface RecordedCall {
  table: string;
  op: "upsert" | "insert" | "select";
  payload: unknown;
  opts?: unknown;
}
let calls: RecordedCall[] = [];
/**
 * PER-TABLE write failures (review round 1 MF1): a map so a test can fail
 * exactly one write in the chain — the partial-failure discipline
 * ("synced only after ALL of a session's writes land") is unprovable with
 * a uniform all-writes-fail switch.
 */
let failWrites: Map<string, { message: string }> = new Map();
const ALL_WRITE_TABLES = [
  "fact_session",
  "fact_phrase",
  "fact_tool_event",
  "dim_material",
  "fact_exercise",
];
function failAllWrites(message: string): void {
  failWrites = new Map(ALL_WRITE_TABLES.map((t) => [t, { message }]));
}
/** What the dim_material read-back returns. */
let materialRows: Array<{ material_id: string; spec_hash: string | null }> = [];
/** The cloud uuid fact_session upserts resolve to. */
const cloudSessionUuid = "cloud-sess-1";

function upserts(table: string): RecordedCall[] {
  return calls.filter((c) => c.table === table && c.op === "upsert");
}

vi.mock("../lib/supabase", () => ({
  supabase: {
    from: (table: string) => ({
      upsert: (payload: unknown, opts?: unknown) => {
        calls.push({ table, op: "upsert", payload, opts });
        const fail = failWrites.get(table) ?? null;
        const result = fail
          ? { data: null, error: fail }
          : { data: null, error: null };
        const promise = Promise.resolve(result);
        return {
          then: promise.then.bind(promise),
          catch: promise.catch.bind(promise),
          select: () => ({
            single: async () =>
              fail
                ? { data: null, error: fail }
                : { data: { session_id: cloudSessionUuid }, error: null },
          }),
        };
      },
      insert: (payload: unknown) => {
        calls.push({ table, op: "insert", payload });
        const fail = failWrites.get(table) ?? null;
        return {
          select: () => ({
            single: async () =>
              fail
                ? { data: null, error: fail }
                : { data: { material_id: "mat-score-1" }, error: null },
          }),
        };
      },
      select: (cols: string) => {
        calls.push({ table, op: "select", payload: cols });
        return {
          eq: () => ({
            eq: () => ({
              limit: () => ({
                maybeSingle: async () => ({ data: null, error: null }),
              }),
            }),
          }),
          in: async () => ({ data: materialRows, error: null }),
        };
      },
    }),
  },
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
    started_at: "2026-07-20T10:00:00Z",
    duration_secs: 600,
    phrase_count: 2,
    played_secs: 240,
    note_count: 180,
    silence_ratio: 0.6,
  };
}

function projection(id: string): SessionProjectionDto {
  return {
    session: {
      id,
      started_at: "2026-07-20T10:00:00Z",
      ended_at: "2026-07-20T10:10:00Z",
      duration_secs: 600,
      phrase_count: 2,
      instrument: "Trumpet",
      practice_mode: "Practice",
      app_version: "2.29.0",
      played_secs: 240,
      note_count: 180,
      silence_ratio: 0.6,
      fingerprint: null,
      score: null,
    },
    phrases: [
      {
        phrase_index: 0,
        start_secs: 1.5,
        end_secs: 4.0,
        note_count: 12,
        stability: 0.8,
        tone: null,
        key_name: "G Mixolydian",
      },
    ],
    events: [
      {
        device_event_id: 7,
        at_secs: 3.0,
        kind: "pocket_start",
        params_json: '{"bpm":90,"mode":"anchor","count_in":true}',
      },
    ],
  };
}

function exerciseFact(
  id: number,
  specHash: string,
  tonic: number,
): ExerciseFactRow {
  return {
    id,
    logged_at: "2026-07-20T10:05:00Z",
    source: "opener",
    label: "Minor triad, enclosed, descending",
    spec_hash: specHash,
    difficulty: 2,
    tonic,
    accuracy: 0.9,
  };
}

function wireInvoke(
  summaries: SessionSummaryDto[],
  facts: ExerciseFactRow[],
): void {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === "get_session_history") return summaries;
    if (cmd === "get_session_projection") {
      const { session_id } = args as { session_id: string };
      return projection(session_id);
    }
    if (cmd === "list_exercise_facts") {
      const { after_id } = args as { after_id: number };
      return facts.filter((f) => f.id > after_id);
    }
    throw new Error(`unexpected command ${cmd}`);
  });
}

const WATERMARK_KEY = "ai-music-companion:dashboard-exercise-watermark:user-1";

describe("syncDashboard — gating matrix (AC1)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    calls = [];
    failWrites = new Map();
  });

  it.each([
    ["signed out", null, true, true],
    ["cloud sync off", "user-1", false, true],
    ["dashboard opt-in off (honest absence)", "user-1", true, false],
  ] as const)(
    "is a hard no-op when %s: nothing read, nothing leaves",
    async (_label, userId, cloudOn, dashOn) => {
      wireInvoke([summary("s1")], [exerciseFact(1, "h1", 0)]);
      const useStore = await freshStore();

      await useStore.getState().syncDashboard(userId, cloudOn, dashOn);

      expect(mockInvoke).not.toHaveBeenCalled();
      expect(calls).toHaveLength(0);
      expect(useStore.getState().dashboardStatus).toBe("idle");
    },
  );

  it("all-on actually syncs (the matrix's fourth row is not a no-op)", async () => {
    wireInvoke([summary("s1")], []);
    const useStore = await freshStore();
    await useStore.getState().syncDashboard("user-1", true, true);
    expect(useStore.getState().dashboardStatus).toBe("synced");
    expect(upserts("fact_session")).toHaveLength(1);
  });
});

describe("syncDashboard — the projection pushes (AC2/AC3)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    calls = [];
    failWrites = new Map();
    materialRows = [{ material_id: "mat-1", spec_hash: "h1" }];
  });

  it("projects fact_session column-for-column on the device-id conflict key", async () => {
    wireInvoke([summary("s1")], []);
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);

    const [call] = upserts("fact_session");
    expect(call.opts).toEqual({ onConflict: "student_id,device_session_id" });
    // Column-for-column against migration 0006 fact_session (a drifted
    // name here = silent 400s on the real stack).
    expect(call.payload).toEqual({
      student_id: "user-1",
      device_session_id: "s1",
      started_at: "2026-07-20T10:00:00Z",
      ended_at: "2026-07-20T10:10:00Z",
      duration_secs: 600,
      played_secs: 240,
      note_count: 180,
      silence_ratio: 0.6,
      phrase_count: 2,
      instrument: "Trumpet",
      practice_mode: "Practice",
      score_material_id: null,
      fingerprint: null,
      app_version: "2.29.0",
    });
    expect(useStore.getState().dashboardSyncedThisRun).toBe(1);
  });

  it("pushes thin phrases and tool events keyed by the CLOUD session uuid", async () => {
    wireInvoke([summary("s1")], []);
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);

    const [phrases] = upserts("fact_phrase");
    expect(phrases.opts).toEqual({
      onConflict: "session_id,phrase_index",
      ignoreDuplicates: true,
    });
    expect(phrases.payload).toEqual([
      {
        session_id: "cloud-sess-1",
        phrase_index: 0,
        start_secs: 1.5,
        end_secs: 4.0,
        note_count: 12,
        stability: 0.8,
        tone: null,
        key_name: "G Mixolydian",
      },
    ]);

    const [events] = upserts("fact_tool_event");
    expect(events.opts).toEqual({
      onConflict: "session_id,device_event_id",
      ignoreDuplicates: true,
    });
    expect(events.payload).toEqual([
      {
        session_id: "cloud-sess-1",
        student_id: "user-1",
        device_event_id: 7,
        at_secs: 3.0,
        kind: "pocket_start",
        params: { bpm: 90, mode: "anchor", count_in: true },
      },
    ]);
  });

  it("a score session lazily creates its dim_material score row and links it (P1)", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_session_history") return [summary("s1")];
      if (cmd === "get_session_projection") {
        const p = projection("s1");
        p.session.score = { score_id: "sc-1", title: "Haydn Concerto, mvt 1" };
        return p;
      }
      if (cmd === "list_exercise_facts") return [];
      throw new Error(`unexpected command ${cmd}`);
    });
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);

    // No unique key on score_id (0006) → select-first, insert-if-absent.
    const inserts = calls.filter(
      (c) => c.table === "dim_material" && c.op === "insert",
    );
    expect(inserts).toHaveLength(1);
    expect(inserts[0].payload).toEqual({
      score_id: "sc-1",
      label: "Haydn Concerto, mvt 1",
      source: "score_practice",
      kind: "score",
    });
    const [session] = upserts("fact_session");
    expect(session.payload).toMatchObject({
      score_material_id: "mat-score-1",
    });
  });

  it("pushes exercises through the dim_material spec_hash upsert (P3)", async () => {
    wireInvoke([], [exerciseFact(1, "h1", 0), exerciseFact(2, "h1", 7)]);
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);

    const [dim] = upserts("dim_material");
    expect(dim.opts).toEqual({
      onConflict: "spec_hash",
      ignoreDuplicates: true,
    });
    // One cell in two keys = ONE dimension row (RV grain, doc §3).
    expect(dim.payload).toEqual([
      {
        spec_hash: "h1",
        label: "Minor triad, enclosed, descending",
        source: "opener",
        kind: "cell",
      },
    ]);

    const [ex] = upserts("fact_exercise");
    expect(ex.opts).toEqual({
      onConflict: "student_id,device_log_id",
      ignoreDuplicates: true,
    });
    expect(ex.payload).toEqual([
      {
        student_id: "user-1",
        device_log_id: 1,
        logged_at: "2026-07-20T10:05:00Z",
        material_id: "mat-1",
        tonic: 0,
        difficulty: 2,
        accuracy: 0.9,
      },
      {
        student_id: "user-1",
        device_log_id: 2,
        logged_at: "2026-07-20T10:05:00Z",
        material_id: "mat-1",
        tonic: 7,
        difficulty: 2,
        accuracy: 0.9,
      },
    ]);
  });
});

describe("syncDashboard — idempotency and the watermarks (AC5/AC6)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    calls = [];
    failWrites = new Map();
    materialRows = [{ material_id: "mat-1", spec_hash: "h1" }];
  });

  it("a second run pushes nothing (session set + exercise watermark hold)", async () => {
    wireInvoke([summary("s1")], [exerciseFact(1, "h1", 0)]);
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);
    expect(upserts("fact_session")).toHaveLength(1);
    expect(store[WATERMARK_KEY]).toBe("1");

    calls = [];
    await useStore.getState().syncDashboard("user-1", true, true);
    expect(calls.filter((c) => c.op !== "select")).toHaveLength(0);
    // The incremental read asked only for rows past the watermark.
    expect(mockInvoke).toHaveBeenLastCalledWith("list_exercise_facts", {
      after_id: 1,
    });
    expect(useStore.getState().dashboardStatus).toBe("synced");
    expect(useStore.getState().dashboardSyncedThisRun).toBe(0);
  });

  it("never advances the watermark past an unresolved material hash", async () => {
    // h2 never resolves to a dimension row — its facts must resurface.
    materialRows = [{ material_id: "mat-1", spec_hash: "h1" }];
    wireInvoke([], [exerciseFact(1, "h1", 0), exerciseFact(2, "h2", 3)]);
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);

    const [ex] = upserts("fact_exercise");
    expect(ex.payload).toHaveLength(1); // only the resolved row went up
    expect(store[WATERMARK_KEY]).toBe("1"); // NOT 2 — id 2 retries next run
  });
});

describe("syncDashboard — calm failure (AC7)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorageMock.clear();
    calls = [];
    failWrites = new Map();
    materialRows = [{ material_id: "mat-1", spec_hash: "h1" }];
  });

  it("a supabase failure sets the dashboard status, marks nothing synced, and retries clean", async () => {
    wireInvoke([summary("s1")], [exerciseFact(1, "h1", 0)]);
    failAllWrites("rls denied");
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);
    expect(useStore.getState().dashboardStatus).toBe("error");
    expect(useStore.getState().dashboardError).toContain("rls denied");
    // Nothing was remembered: no session set, no watermark advance.
    expect(store[WATERMARK_KEY]).toBeUndefined();
    // The legacy session-sync status is independent and untouched.
    expect(useStore.getState().status).toBe("idle");

    // Next trigger retries the same rows and succeeds.
    failWrites = new Map();
    await useStore.getState().syncDashboard("user-1", true, true);
    expect(useStore.getState().dashboardStatus).toBe("synced");
    expect(useStore.getState().dashboardSyncedThisRun).toBe(1);
    expect(store[WATERMARK_KEY]).toBe("1");
  });

  // Review round 1 MF1: the all-three-writes-or-nothing discipline. A
  // uniform failure can't prove it — here fact_session LANDS but the
  // fact_phrase child write fails, and the session must still NOT enter
  // the synced set (a mutant that marks synced right after the session
  // upsert dies here), then fully re-push on the next run.
  it("a partial failure (session landed, phrases failed) does not mark the session synced and retries ALL writes", async () => {
    wireInvoke([summary("s1")], []);
    failWrites = new Map([["fact_phrase", { message: "phrase write lost" }]]);
    const useStore = await freshStore();

    await useStore.getState().syncDashboard("user-1", true, true);
    // The session write itself went through…
    expect(upserts("fact_session")).toHaveLength(1);
    // …but the run failed, and the synced set must NOT contain s1.
    expect(useStore.getState().dashboardStatus).toBe("error");
    expect(useStore.getState().dashboardError).toContain("phrase write lost");
    const syncedSet =
      store["ai-music-companion:dashboard-synced-sessions:user-1"];
    expect(syncedSet ?? "[]").not.toContain("s1");
    expect(useStore.getState().dashboardSyncedThisRun).toBe(0);

    // Next run: the WHOLE session re-pushes (session + phrases + events),
    // absorbed by the device-id conflict keys, and only now is remembered.
    calls = [];
    failWrites = new Map();
    await useStore.getState().syncDashboard("user-1", true, true);
    expect(upserts("fact_session")).toHaveLength(1);
    expect(upserts("fact_phrase")).toHaveLength(1);
    expect(upserts("fact_tool_event")).toHaveLength(1);
    expect(useStore.getState().dashboardStatus).toBe("synced");
    expect(
      store["ai-music-companion:dashboard-synced-sessions:user-1"],
    ).toContain("s1");
  });
});

describe("dashboard payload builders — the structural privacy pin (AC4/AC9)", () => {
  it("built exercise/material rows never contain spec_json or seed keys", async () => {
    const { buildFactExerciseRows, buildDimMaterialRows } =
      await import("./syncStore");
    const facts = [exerciseFact(1, "h1", 0)];
    const byHash = new Map([["h1", "mat-1"]]);
    for (const row of [
      ...buildFactExerciseRows("user-1", facts, byHash),
      ...buildDimMaterialRows(facts),
    ] as Array<Record<string, unknown>>) {
      expect(Object.keys(row)).not.toContain("spec_json");
      expect(Object.keys(row)).not.toContain("seed");
    }
  });

  it("built phrase rows carry only the thin doc-§2 fields (no onsets/curves)", async () => {
    const { buildFactPhraseRows } = await import("./syncStore");
    const rows = buildFactPhraseRows("cloud-1", projection("s1").phrases);
    expect(Object.keys(rows[0]).sort()).toEqual([
      "end_secs",
      "key_name",
      "note_count",
      "phrase_index",
      "session_id",
      "stability",
      "start_secs",
      "tone",
    ]);
  });

  // Review round 1 MF2: `JSON.parse("null")` SUCCEEDS — a raw passthrough
  // would push `params: null` into a NOT NULL column (0006 L599) and wedge
  // that session out of the synced set on every run, forever. Any
  // non-object JSON coerces to {}; so does garbage.
  it("tool-event params coerce non-object JSON (null, arrays, scalars) and garbage to {}", async () => {
    const { buildFactToolEventRows } = await import("./syncStore");
    const event = (id: number, paramsJson: string) => ({
      device_event_id: id,
      at_secs: 1.0,
      kind: "band_stop",
      params_json: paramsJson,
    });
    const rows = buildFactToolEventRows("cloud-1", "user-1", [
      event(1, "null"), // valid JSON, not an object — the wedge case
      event(2, "[1,2]"), // valid JSON array
      event(3, '"free"'), // valid JSON string
      event(4, "not json at all"), // parse failure
      event(5, '{"bpm":92}'), // the real vocabulary passes through
    ]);
    expect(rows.map((r) => r.params)).toEqual([{}, {}, {}, {}, { bpm: 92 }]);
  });

  it("documents the #470 narration-flag semantics at the projection site (option b)", () => {
    // The flag means "an LLM response parsed", NOT "the shown text was
    // LLM-authored" — the caveat must live where the event is projected,
    // so T4 dashboard work reads it before phrasing any UI copy.
    // vitest runs with cwd = apps/desktop; jsdom's import.meta.url is not a
    // file: URL, so resolve from the working directory instead.
    const src = readFileSync(
      join(process.cwd(), "src", "stores", "syncStore.ts"),
      "utf8",
    );
    expect(src).toMatch(/an LLM response parsed/);
    expect(src).toMatch(/NOT "the shown recap text was LLM-authored"/);
    expect(src).toMatch(/#470/);
  });
});
