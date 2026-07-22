import { create } from "zustand";
import { supabase } from "../lib/supabase";

/**
 * #449 enrollment slice — classroom enrollment over the 0006 schema
 * (spec: docs/specs/449-enrollment.md).
 *
 * This store owns EVERY network call of the enrollment flow, and nothing
 * else. Discipline (same as syncStore):
 *   - Signed-in gated: every action takes the userId and is a hard no-op
 *     without one. Nothing here ever runs in the background — each call is
 *     behind an explicit user action (disclosed in ConnectionsPrivacy and
 *     the offline-first enumeration table).
 *   - Calm failures: errors land in state as plain messages; nothing throws
 *     into the UI. Offline is normal life.
 *   - No business logic: activation, the consent CHECK, the under-13
 *     parent-only gate, code expiry and revoked-row refusal are all enforced
 *     server-side by migration 0006 (RLS + SECURITY DEFINER RPCs). The FE
 *     mirrors them for honest UX; the schema is the truth.
 *
 * Honest absence: students cannot select `classrooms` rows (RLS — the row
 * carries the live join code), so the student-side list shows the enrollment
 * itself (status, joined date, consent party) and never invents a classroom
 * name. Silence > lies.
 */

export type AgeTier = "under_13" | "teen_13_17" | "adult";
export type ConsentParty = "student" | "parent";

export interface EnrollmentRow {
  id: string;
  classroom_id: string;
  status: string;
  consent: string | null;
  consented_at: string | null;
  joined_via_code_at: string | null;
  created_at: string;
}

export interface ClassroomRow {
  id: string;
  name: string;
  join_code: string | null;
  join_code_expires_at: string | null;
  created_at: string;
}

export interface EnrollmentProfile {
  role: string;
  ageTier: AgeTier | null;
}

export interface EnrollmentState {
  /** Own profile (role + age tier) — drives the teacher card + consent path. */
  profile: EnrollmentProfile | null;
  /** Own enrollment rows (student view). */
  enrollments: EnrollmentRow[];
  /** Own classrooms (teacher view). */
  classrooms: ClassroomRow[];
  /** classroom_id → count of ACTIVE enrollments (teacher roster count). */
  rosterCounts: Record<string, number>;
  /** A round-trip is in flight. */
  working: boolean;
  /** Calm, human-readable failure of the last action (null = fine). */
  error: string | null;

  loadProfile: (userId: string | undefined) => Promise<void>;
  setAgeTier: (userId: string | undefined, tier: AgeTier) => Promise<boolean>;
  loadEnrollments: (userId: string | undefined) => Promise<void>;
  redeemJoinCode: (
    userId: string | undefined,
    code: string,
    consent: ConsentParty,
  ) => Promise<boolean>;
  leaveClassroom: (
    userId: string | undefined,
    enrollmentId: string,
  ) => Promise<boolean>;
  loadClassrooms: (userId: string | undefined) => Promise<void>;
  createClassroom: (
    userId: string | undefined,
    name: string,
  ) => Promise<boolean>;
  issueJoinCode: (
    userId: string | undefined,
    classroomId: string,
  ) => Promise<boolean>;
  clearError: () => void;
}

function message(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

function toAgeTier(value: string | null): AgeTier | null {
  return value === "under_13" || value === "teen_13_17" || value === "adult"
    ? value
    : null;
}

export const useEnrollmentStore = create<EnrollmentState>((set, get) => ({
  profile: null,
  enrollments: [],
  classrooms: [],
  rosterCounts: {},
  working: false,
  error: null,

  loadProfile: async (userId) => {
    if (!userId) return;
    try {
      const { data, error } = await supabase
        .from("profiles")
        .select("role, age_tier")
        .eq("id", userId)
        .maybeSingle();
      if (error) {
        set({ error: error.message });
        return;
      }
      if (data) {
        set({
          profile: { role: data.role, ageTier: toAgeTier(data.age_tier) },
        });
      }
    } catch (e) {
      set({ error: message(e) });
    }
  },

  // The age step (spec AC3): recorded BEFORE consent is offered, so the
  // server-side under-13 gate in redeem_join_code binds against a real tier.
  // Age group only — never a birthdate (0003's rule).
  setAgeTier: async (userId, tier) => {
    if (!userId) return false;
    set({ working: true, error: null });
    try {
      const { error } = await supabase
        .from("profiles")
        .update({ age_tier: tier })
        .eq("id", userId);
      if (error) {
        set({ working: false, error: error.message });
        return false;
      }
      set((s) => ({
        working: false,
        profile: s.profile
          ? { ...s.profile, ageTier: tier }
          : { role: "student", ageTier: tier },
      }));
      return true;
    } catch (e) {
      set({ working: false, error: message(e) });
      return false;
    }
  },

  loadEnrollments: async (userId) => {
    if (!userId) return;
    try {
      const { data, error } = await supabase
        .from("enrollments")
        .select(
          "id, classroom_id, status, consent, consented_at, joined_via_code_at, created_at",
        )
        .eq("student_id", userId);
      if (error) {
        set({ error: error.message });
        return;
      }
      set({ enrollments: data ?? [] });
    } catch (e) {
      set({ error: message(e) });
    }
  },

  // THE only path to status='active' (0006). Called exclusively from the
  // consent screen's explicit accept — spec AC1: no accept, no call.
  redeemJoinCode: async (userId, code, consent) => {
    if (!userId) return false;
    set({ working: true, error: null });
    try {
      const { error } = await supabase.rpc("redeem_join_code", {
        p_code: code.trim(),
        p_consent: consent,
      });
      if (error) {
        set({ working: false, error: error.message });
        return false;
      }
      set({ working: false });
      await get().loadEnrollments(userId);
      return true;
    } catch (e) {
      set({ working: false, error: message(e) });
      return false;
    }
  },

  // Self-serve leave — the enrollments_update_student_revoke path. The guard
  // trigger freezes every column except status/revoked_at for students, and a
  // revoked row grants the teacher nothing on the very next query.
  leaveClassroom: async (userId, enrollmentId) => {
    if (!userId) return false;
    set({ working: true, error: null });
    try {
      const { error } = await supabase
        .from("enrollments")
        .update({ status: "revoked", revoked_at: new Date().toISOString() })
        .eq("id", enrollmentId)
        .eq("student_id", userId);
      if (error) {
        set({ working: false, error: error.message });
        return false;
      }
      set({ working: false });
      await get().loadEnrollments(userId);
      return true;
    } catch (e) {
      set({ working: false, error: message(e) });
      return false;
    }
  },

  loadClassrooms: async (userId) => {
    if (!userId) return;
    try {
      const { data, error } = await supabase
        .from("classrooms")
        .select("id, name, join_code, join_code_expires_at, created_at")
        .eq("teacher_id", userId);
      if (error) {
        set({ error: error.message });
        return;
      }
      const classrooms = data ?? [];
      // Roster counts: active enrollments only. RLS already scopes the read
      // to rows the caller may see; the client additionally counts only the
      // caller's OWN classrooms, so a teacher's own student-side enrollments
      // elsewhere never inflate a roster.
      const own = new Set(classrooms.map((c) => c.id));
      const rosterCounts: Record<string, number> = {};
      const { data: enrollmentRows, error: enrollmentError } = await supabase
        .from("enrollments")
        .select("classroom_id, status")
        .eq("status", "active");
      if (enrollmentError) {
        set({ classrooms, error: enrollmentError.message });
        return;
      }
      for (const row of enrollmentRows ?? []) {
        if (own.has(row.classroom_id)) {
          rosterCounts[row.classroom_id] =
            (rosterCounts[row.classroom_id] ?? 0) + 1;
        }
      }
      set({ classrooms, rosterCounts });
    } catch (e) {
      set({ error: message(e) });
    }
  },

  createClassroom: async (userId, name) => {
    if (!userId) return false;
    const trimmed = name.trim();
    if (!trimmed) return false;
    set({ working: true, error: null });
    try {
      const { error } = await supabase
        .from("classrooms")
        .insert({ teacher_id: userId, name: trimmed });
      if (error) {
        set({ working: false, error: error.message });
        return false;
      }
      set({ working: false });
      await get().loadClassrooms(userId);
      return true;
    } catch (e) {
      set({ working: false, error: message(e) });
      return false;
    }
  },

  // Mint/rotate, then RE-READ the row: the displayed code + expiry come from
  // the classroom row (server clock truth), never a client-side TTL guess.
  issueJoinCode: async (userId, classroomId) => {
    if (!userId) return false;
    set({ working: true, error: null });
    try {
      const { error } = await supabase.rpc("issue_join_code", {
        p_classroom_id: classroomId,
      });
      if (error) {
        set({ working: false, error: error.message });
        return false;
      }
      set({ working: false });
      await get().loadClassrooms(userId);
      return true;
    } catch (e) {
      set({ working: false, error: message(e) });
      return false;
    }
  },

  clearError: () => set({ error: null }),
}));
