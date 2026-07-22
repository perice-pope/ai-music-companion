import { describe, it, expect, beforeEach, vi } from "vitest";

/**
 * #449 enrollment slice (spec docs/specs/449-enrollment.md) — the store that
 * owns every enrollment network call. Covers the RPC contracts (AC1/AC5),
 * the student self-revoke (AC4), the age-tier write (AC3), roster counting
 * and the TTL-from-the-row discipline (AC5), signed-out no-ops and calm
 * failures (AC7).
 */

interface RecordedCall {
  table: string;
  op: "select" | "update" | "insert";
  payload?: unknown;
  filters: Array<[string, unknown]>;
}
let calls: RecordedCall[] = [];
let rpcCalls: Array<{ fn: string; args: unknown }> = [];
/** Per-table responses for awaited select chains. */
let selectData: Record<string, unknown[]> = {};
/** Per-table failure injection. */
let failTables: Map<string, { message: string }> = new Map();
let failRpc: { message: string } | null = null;
/** profiles maybeSingle() payload. */
let profileRow: { role: string; age_tier: string | null } | null = {
  role: "student",
  age_tier: "adult",
};

function result(table: string, data: unknown) {
  const fail = failTables.get(table);
  return fail ? { data: null, error: fail } : { data, error: null };
}

vi.mock("../lib/supabase", () => ({
  supabase: {
    rpc: (fn: string, args: unknown) => {
      rpcCalls.push({ fn, args });
      return Promise.resolve(
        failRpc
          ? { data: null, error: failRpc }
          : { data: "uuid", error: null },
      );
    },
    from: (table: string) => ({
      select: (_cols: string) => {
        const call: RecordedCall = { table, op: "select", filters: [] };
        calls.push(call);
        const chain = {
          eq: (col: string, value: unknown) => {
            call.filters.push([col, value]);
            return chain;
          },
          maybeSingle: async () => result(table, profileRow),
          then: (
            onOk: (v: unknown) => unknown,
            onErr?: (e: unknown) => unknown,
          ) =>
            Promise.resolve(result(table, selectData[table] ?? [])).then(
              onOk,
              onErr,
            ),
        };
        return chain;
      },
      update: (payload: unknown) => {
        const call: RecordedCall = {
          table,
          op: "update",
          payload,
          filters: [],
        };
        calls.push(call);
        const chain = {
          eq: (col: string, value: unknown) => {
            call.filters.push([col, value]);
            return chain;
          },
          then: (
            onOk: (v: unknown) => unknown,
            onErr?: (e: unknown) => unknown,
          ) => Promise.resolve(result(table, null)).then(onOk, onErr),
        };
        return chain;
      },
      insert: (payload: unknown) => {
        calls.push({ table, op: "insert", payload, filters: [] });
        return Promise.resolve(result(table, null));
      },
    }),
  },
}));

async function freshStore() {
  vi.resetModules();
  const mod = await import("./enrollmentStore");
  return mod.useEnrollmentStore;
}

beforeEach(() => {
  calls = [];
  rpcCalls = [];
  selectData = {};
  failTables = new Map();
  failRpc = null;
  profileRow = { role: "student", age_tier: "adult" };
});

describe("enrollmentStore — signed-out is a hard no-op (AC7)", () => {
  it("makes zero supabase calls without a userId", async () => {
    const useStore = await freshStore();
    const s = useStore.getState();
    await s.loadProfile(undefined);
    await s.loadEnrollments(undefined);
    await s.loadClassrooms(undefined);
    expect(await s.redeemJoinCode(undefined, "ABCD2345", "student")).toBe(
      false,
    );
    expect(await s.leaveClassroom(undefined, "e-1")).toBe(false);
    expect(await s.createClassroom(undefined, "Jazz Band")).toBe(false);
    expect(await s.issueJoinCode(undefined, "c-1")).toBe(false);
    expect(await s.setAgeTier(undefined, "adult")).toBe(false);
    expect(calls).toHaveLength(0);
    expect(rpcCalls).toHaveLength(0);
  });
});

describe("enrollmentStore — redeem (AC1)", () => {
  it("calls redeem_join_code with the trimmed code and consent party, then reloads enrollments", async () => {
    selectData["enrollments"] = [
      {
        id: "e-1",
        classroom_id: "c-1",
        status: "active",
        consent: "student",
        consented_at: "2026-07-21T00:00:00Z",
        joined_via_code_at: "2026-07-21T00:00:00Z",
        created_at: "2026-07-21T00:00:00Z",
      },
    ];
    const useStore = await freshStore();
    const ok = await useStore
      .getState()
      .redeemJoinCode("u-1", "  ABCD2345 ", "student");
    expect(ok).toBe(true);
    expect(rpcCalls).toEqual([
      {
        fn: "redeem_join_code",
        args: { p_code: "ABCD2345", p_consent: "student" },
      },
    ]);
    expect(useStore.getState().enrollments).toHaveLength(1);
    expect(useStore.getState().error).toBeNull();
  });

  it("passes consent='parent' through unchanged (the under-13 path)", async () => {
    const useStore = await freshStore();
    await useStore.getState().redeemJoinCode("u-1", "ABCD2345", "parent");
    expect(rpcCalls[0]?.args).toEqual({
      p_code: "ABCD2345",
      p_consent: "parent",
    });
  });

  it("a failed redeem is calm: error message stored, returns false, no throw (AC7)", async () => {
    failRpc = { message: "invalid or expired join code" };
    const useStore = await freshStore();
    const ok = await useStore
      .getState()
      .redeemJoinCode("u-1", "WRONGCODE", "student");
    expect(ok).toBe(false);
    expect(useStore.getState().error).toBe("invalid or expired join code");
    expect(useStore.getState().working).toBe(false);
    // No enrollment reload after a failed redeem.
    expect(calls.filter((c) => c.table === "enrollments")).toHaveLength(0);
  });
});

describe("enrollmentStore — leave / revoke (AC4)", () => {
  it("updates the student's own row to revoked with a revoked_at stamp", async () => {
    const useStore = await freshStore();
    const ok = await useStore.getState().leaveClassroom("u-1", "e-9");
    expect(ok).toBe(true);
    const update = calls.find(
      (c) => c.table === "enrollments" && c.op === "update",
    );
    expect(update).toBeDefined();
    expect(update?.payload).toMatchObject({ status: "revoked" });
    expect((update?.payload as { revoked_at: string }).revoked_at).toBeTruthy();
    expect(update?.filters).toEqual([
      ["id", "e-9"],
      ["student_id", "u-1"],
    ]);
  });

  it("a failed revoke keeps the error calm and returns false", async () => {
    failTables.set("enrollments", { message: "network down" });
    const useStore = await freshStore();
    expect(await useStore.getState().leaveClassroom("u-1", "e-9")).toBe(false);
    expect(useStore.getState().error).toBe("network down");
  });
});

describe("enrollmentStore — profile + age tier (AC3)", () => {
  it("loads role and age_tier from the own profile row", async () => {
    profileRow = { role: "teacher", age_tier: "under_13" };
    const useStore = await freshStore();
    await useStore.getState().loadProfile("u-1");
    expect(useStore.getState().profile).toEqual({
      role: "teacher",
      ageTier: "under_13",
    });
    const sel = calls.find((c) => c.table === "profiles");
    expect(sel?.filters).toEqual([["id", "u-1"]]);
  });

  it("setAgeTier writes profiles.age_tier for the own row only", async () => {
    const useStore = await freshStore();
    const ok = await useStore.getState().setAgeTier("u-1", "under_13");
    expect(ok).toBe(true);
    const update = calls.find(
      (c) => c.table === "profiles" && c.op === "update",
    );
    expect(update?.payload).toEqual({ age_tier: "under_13" });
    expect(update?.filters).toEqual([["id", "u-1"]]);
    expect(useStore.getState().profile?.ageTier).toBe("under_13");
  });
});

describe("enrollmentStore — teacher side (AC5)", () => {
  it("createClassroom inserts a classroom owned by the caller and refreshes", async () => {
    selectData["classrooms"] = [];
    selectData["enrollments"] = [];
    const useStore = await freshStore();
    const ok = await useStore.getState().createClassroom("t-1", "  Jazz Band ");
    expect(ok).toBe(true);
    const insert = calls.find(
      (c) => c.table === "classrooms" && c.op === "insert",
    );
    expect(insert?.payload).toEqual({ teacher_id: "t-1", name: "Jazz Band" });
  });

  it("issueJoinCode calls the rpc for the classroom, then re-reads rows (TTL truth from the row, not the client)", async () => {
    selectData["classrooms"] = [
      {
        id: "c-1",
        name: "Jazz Band",
        join_code: "ABCD2345",
        join_code_expires_at: "2026-07-28T00:00:00Z",
        created_at: "2026-07-01T00:00:00Z",
      },
    ];
    selectData["enrollments"] = [];
    const useStore = await freshStore();
    const ok = await useStore.getState().issueJoinCode("t-1", "c-1");
    expect(ok).toBe(true);
    expect(rpcCalls).toEqual([
      { fn: "issue_join_code", args: { p_classroom_id: "c-1" } },
    ]);
    // The refresh happened and carried the server-side code + expiry.
    expect(useStore.getState().classrooms[0]?.join_code).toBe("ABCD2345");
    expect(useStore.getState().classrooms[0]?.join_code_expires_at).toBe(
      "2026-07-28T00:00:00Z",
    );
  });

  it("loadClassrooms counts only ACTIVE enrollments in the teacher's own classrooms", async () => {
    selectData["classrooms"] = [
      {
        id: "c-1",
        name: "Jazz Band",
        join_code: null,
        join_code_expires_at: null,
        created_at: "2026-07-01T00:00:00Z",
      },
    ];
    // Two active rows in c-1, one active row in someone ELSE's classroom
    // (the teacher's own student-side enrollment) — must not count.
    selectData["enrollments"] = [
      { classroom_id: "c-1", status: "active" },
      { classroom_id: "c-1", status: "active" },
      { classroom_id: "other", status: "active" },
    ];
    const useStore = await freshStore();
    await useStore.getState().loadClassrooms("t-1");
    expect(useStore.getState().rosterCounts).toEqual({ "c-1": 2 });
    // The enrollment read itself asks only for active rows.
    const enrollmentSelect = calls.find(
      (c) => c.table === "enrollments" && c.op === "select",
    );
    expect(enrollmentSelect?.filters).toEqual([["status", "active"]]);
  });

  it("a failed issue rpc is calm (AC7)", async () => {
    failRpc = { message: "classroom not found or not owned by you" };
    const useStore = await freshStore();
    expect(await useStore.getState().issueJoinCode("t-1", "c-x")).toBe(false);
    expect(useStore.getState().error).toBe(
      "classroom not found or not owned by you",
    );
  });
});
