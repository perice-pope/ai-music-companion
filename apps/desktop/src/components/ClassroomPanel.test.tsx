import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { User } from "@supabase/supabase-js";

/**
 * #449 enrollment slice (spec docs/specs/449-enrollment.md) — the flow.
 * The consent screen gates the RPC (AC1), the under-13 parent/guardian
 * branch (AC2), the age step (AC3), Leave/revoke (AC4), the teacher card
 * (AC5), the sync prompt (AC6), and calm failures + signed-out silence
 * (AC7). Real stores, mocked Supabase — these are behavior tests against
 * the wire calls.
 */

let rpcCalls: Array<{ fn: string; args: unknown }> = [];
let failRpc: { message: string } | null = null;
let profileRow: { role: string; age_tier: string | null } | null = {
  role: "student",
  age_tier: "adult",
};
let enrollmentRows: unknown[] = [];
let classroomRows: unknown[] = [];
let updateCalls: Array<{
  table: string;
  payload: unknown;
  filters: Array<[string, unknown]>;
}> = [];
let selectCount = 0;

vi.mock("../lib/supabase", () => ({
  supabase: {
    auth: {
      onAuthStateChange: () => ({
        data: { subscription: { unsubscribe: () => undefined } },
      }),
    },
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
        selectCount += 1;
        const data =
          table === "enrollments"
            ? enrollmentRows
            : table === "classrooms"
              ? classroomRows
              : [];
        const chain = {
          eq: () => chain,
          maybeSingle: async () => ({ data: profileRow, error: null }),
          then: (
            onOk: (v: unknown) => unknown,
            onErr?: (e: unknown) => unknown,
          ) => Promise.resolve({ data, error: null }).then(onOk, onErr),
        };
        return chain;
      },
      update: (payload: unknown) => {
        const call = {
          table,
          payload,
          filters: [] as Array<[string, unknown]>,
        };
        updateCalls.push(call);
        const chain = {
          eq: (col: string, value: unknown) => {
            call.filters.push([col, value]);
            return chain;
          },
          then: (
            onOk: (v: unknown) => unknown,
            onErr?: (e: unknown) => unknown,
          ) => Promise.resolve({ data: null, error: null }).then(onOk, onErr),
        };
        return chain;
      },
      insert: (_payload: unknown) =>
        Promise.resolve({ data: null, error: null }),
    }),
  },
}));

import ClassroomPanel, {
  CONSENT_NEVER_LINE,
  UNDER_13_NOTICE,
} from "./ClassroomPanel";
import { useAuthStore } from "../stores/authStore";
import { useConnectionsStore } from "../stores/connectionsStore";
import { useEnrollmentStore } from "../stores/enrollmentStore";

const ACTIVE_ENROLLMENT = {
  id: "e-1",
  classroom_id: "c-1",
  status: "active",
  consent: "student",
  consented_at: "2026-07-20T00:00:00Z",
  joined_via_code_at: "2026-07-20T00:00:00Z",
  created_at: "2026-07-20T00:00:00Z",
};

function signIn(id = "u-1") {
  useAuthStore.setState({
    status: "signed_in",
    user: { id } as User,
  });
}

async function openConsentScreen() {
  render(<ClassroomPanel />);
  fireEvent.click(await screen.findByTestId("join-classroom-open"));
  fireEvent.change(screen.getByLabelText("Join code"), {
    target: { value: "ABCD2345" },
  });
  fireEvent.click(screen.getByTestId("join-code-continue"));
  await screen.findByTestId("consent-screen");
}

beforeEach(() => {
  rpcCalls = [];
  failRpc = null;
  profileRow = { role: "student", age_tier: "adult" };
  enrollmentRows = [];
  classroomRows = [];
  updateCalls = [];
  selectCount = 0;
  localStorage.clear();
  useAuthStore.setState({
    status: "signed_out",
    user: null,
    session: null,
    error: null,
  });
  useConnectionsStore.setState({
    cloudSyncEnabled: false,
    teacherSharingEnabled: false,
    dashboardSyncEnabled: false,
    autoUpdateCheckEnabled: false,
  });
  useEnrollmentStore.setState({
    profile: null,
    enrollments: [],
    classrooms: [],
    rosterCounts: {},
    working: false,
    error: null,
  });
});

describe("ClassroomPanel — signed out (AC7)", () => {
  it("renders nothing and makes no supabase call", () => {
    const { container } = render(<ClassroomPanel />);
    expect(container.firstChild).toBeNull();
    expect(selectCount).toBe(0);
    expect(rpcCalls).toHaveLength(0);
  });
});

describe("ClassroomPanel — the consent screen gates the rpc (AC1)", () => {
  it("shows the full disclosure and makes NO redeem call until accept", async () => {
    signIn();
    await openConsentScreen();
    // The plain-words disclosure list (T2 surfaces) + the never-list.
    expect(screen.getByText(/Session recaps/)).toBeInTheDocument();
    expect(screen.getByText(/Session facts/)).toBeInTheDocument();
    expect(screen.getByText(/Phrase timings/)).toBeInTheDocument();
    expect(screen.getByText(/Exercises/)).toBeInTheDocument();
    expect(screen.getByText(/Tools and progress/)).toBeInTheDocument();
    expect(screen.getByText(CONSENT_NEVER_LINE)).toBeInTheDocument();
    // No accept, no call.
    expect(rpcCalls).toHaveLength(0);

    fireEvent.click(screen.getByTestId("consent-accept"));
    await waitFor(() =>
      expect(rpcCalls).toEqual([
        {
          fn: "redeem_join_code",
          args: { p_code: "ABCD2345", p_consent: "student" },
        },
      ]),
    );
  });

  it("cancelling the consent screen never calls redeem", async () => {
    signIn();
    await openConsentScreen();
    fireEvent.click(screen.getByTestId("consent-cancel"));
    expect(screen.queryByTestId("consent-screen")).not.toBeInTheDocument();
    expect(rpcCalls).toHaveLength(0);
  });

  it("a failed redeem shows the calm message and stays recoverable (AC7)", async () => {
    signIn();
    failRpc = { message: "invalid or expired join code" };
    await openConsentScreen();
    fireEvent.click(screen.getByTestId("consent-accept"));
    expect(
      await screen.findByText("invalid or expired join code"),
    ).toBeInTheDocument();
    // Still on the consent screen — the flow did not dead-end or throw.
    expect(screen.getByTestId("consent-screen")).toBeInTheDocument();
    expect(screen.queryByTestId("sync-prompt")).not.toBeInTheDocument();
  });
});

describe("ClassroomPanel — under-13 parent/guardian branch (AC2)", () => {
  it("requires the guardian acknowledgment and redeems with consent='parent'", async () => {
    profileRow = { role: "student", age_tier: "under_13" };
    signIn();
    await openConsentScreen();
    // The screen says a parent/guardian must complete it.
    expect(screen.getByText(UNDER_13_NOTICE)).toBeInTheDocument();
    // No self-consent button exists on this branch.
    expect(screen.queryByTestId("consent-accept")).not.toBeInTheDocument();
    const accept = screen.getByTestId("consent-accept-parent");
    expect(accept).toBeDisabled();
    // Ticking the guardian acknowledgment enables it.
    fireEvent.click(screen.getByTestId("guardian-ack"));
    expect(accept).toBeEnabled();
    fireEvent.click(accept);
    await waitFor(() =>
      expect(rpcCalls).toEqual([
        {
          fn: "redeem_join_code",
          args: { p_code: "ABCD2345", p_consent: "parent" },
        },
      ]),
    );
  });
});

describe("ClassroomPanel — unknown age runs the age step first (AC3)", () => {
  it("persists the chosen tier before consent and routes under-13 into the parent branch", async () => {
    profileRow = { role: "student", age_tier: null };
    signIn();
    render(<ClassroomPanel />);
    fireEvent.click(await screen.findByTestId("join-classroom-open"));
    fireEvent.change(screen.getByLabelText("Join code"), {
      target: { value: "ABCD2345" },
    });
    fireEvent.click(screen.getByTestId("join-code-continue"));
    // Age step, not consent, comes first.
    expect(screen.getByTestId("age-step")).toBeInTheDocument();
    expect(screen.queryByTestId("consent-screen")).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("Under 13"));
    fireEvent.click(screen.getByTestId("age-continue"));
    await screen.findByTestId("consent-screen");
    // The tier was written to the profile (age group only).
    const profileUpdate = updateCalls.find((c) => c.table === "profiles");
    expect(profileUpdate?.payload).toEqual({ age_tier: "under_13" });
    expect(profileUpdate?.filters).toEqual([["id", "u-1"]]);
    // And the consent screen is the parent/guardian branch.
    expect(screen.getByTestId("consent-under-13")).toBeInTheDocument();
    expect(rpcCalls).toHaveLength(0);
  });
});

describe("ClassroomPanel — sync prompt after joining (AC6)", () => {
  it("accepting enables BOTH cloud sync and dashboard sync", async () => {
    signIn();
    await openConsentScreen();
    fireEvent.click(screen.getByTestId("consent-accept"));
    await screen.findByTestId("sync-prompt");
    // Nothing turned on silently by the join itself.
    expect(useConnectionsStore.getState().cloudSyncEnabled).toBe(false);
    expect(useConnectionsStore.getState().dashboardSyncEnabled).toBe(false);
    fireEvent.click(screen.getByTestId("sync-prompt-accept"));
    expect(useConnectionsStore.getState().cloudSyncEnabled).toBe(true);
    expect(useConnectionsStore.getState().dashboardSyncEnabled).toBe(true);
  });

  it("declining leaves both switches off (honest offline absence)", async () => {
    signIn();
    await openConsentScreen();
    fireEvent.click(screen.getByTestId("consent-accept"));
    await screen.findByTestId("sync-prompt");
    fireEvent.click(screen.getByTestId("sync-prompt-decline"));
    expect(useConnectionsStore.getState().cloudSyncEnabled).toBe(false);
    expect(useConnectionsStore.getState().dashboardSyncEnabled).toBe(false);
    expect(screen.queryByTestId("sync-prompt")).not.toBeInTheDocument();
  });
});

describe("ClassroomPanel — enrollments + Leave (AC4)", () => {
  it("lists the enrollment honestly and revokes only after an explicit confirm", async () => {
    enrollmentRows = [ACTIVE_ENROLLMENT];
    signIn();
    render(<ClassroomPanel />);
    expect(
      await screen.findByText("Enrolled in a classroom"),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("leave-e-1"));
    // First click only asks — no wire call yet.
    expect(updateCalls).toHaveLength(0);
    fireEvent.click(screen.getByTestId("confirm-leave-e-1"));
    await waitFor(() => expect(updateCalls.length).toBeGreaterThan(0));
    const update = updateCalls[0];
    expect(update.table).toBe("enrollments");
    expect(update.payload).toMatchObject({ status: "revoked" });
    expect(update.filters).toEqual([
      ["id", "e-1"],
      ["student_id", "u-1"],
    ]);
  });
});

describe("ClassroomPanel — teacher card (AC5)", () => {
  it("shows roster count + the join code with its row-sourced expiry, and mints on demand", async () => {
    profileRow = { role: "teacher", age_tier: "adult" };
    classroomRows = [
      {
        id: "c-1",
        name: "Jazz Band",
        join_code: "ABCD2345",
        join_code_expires_at: "2099-01-01T00:00:00Z",
        created_at: "2026-07-01T00:00:00Z",
      },
    ];
    // Roster truth comes over the wire: three ACTIVE enrollments in c-1.
    enrollmentRows = [
      { ...ACTIVE_ENROLLMENT, id: "e-a", student_id: "s-a" },
      { ...ACTIVE_ENROLLMENT, id: "e-b", student_id: "s-b" },
      { ...ACTIVE_ENROLLMENT, id: "e-c", student_id: "s-c" },
    ];
    signIn("t-1");
    render(<ClassroomPanel />);
    expect(await screen.findByTestId("teacher-card")).toBeInTheDocument();
    expect(await screen.findByText("Jazz Band")).toBeInTheDocument();
    expect(await screen.findByText("3 active students")).toBeInTheDocument();
    expect(screen.getByTestId("join-code-c-1")).toHaveTextContent("ABCD2345");
    expect(screen.getByText(/expires/)).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("issue-code-c-1"));
    await waitFor(() =>
      expect(rpcCalls).toContainEqual({
        fn: "issue_join_code",
        args: { p_classroom_id: "c-1" },
      }),
    );
  });

  it("labels an expired code honestly (never a stale code shown as live)", async () => {
    profileRow = { role: "teacher", age_tier: "adult" };
    classroomRows = [
      {
        id: "c-2",
        name: "Concert Band",
        join_code: "OLDCODE2",
        join_code_expires_at: "2020-01-01T00:00:00Z",
        created_at: "2026-07-01T00:00:00Z",
      },
    ];
    signIn("t-1");
    render(<ClassroomPanel />);
    expect(await screen.findByText("Join code expired.")).toBeInTheDocument();
    expect(screen.queryByTestId("join-code-c-2")).not.toBeInTheDocument();
  });

  it("does not render the teacher card for a student profile", async () => {
    signIn();
    render(<ClassroomPanel />);
    await screen.findByTestId("classroom-panel");
    await waitFor(() =>
      expect(useEnrollmentStore.getState().profile).not.toBeNull(),
    );
    expect(screen.queryByTestId("teacher-card")).not.toBeInTheDocument();
  });
});
