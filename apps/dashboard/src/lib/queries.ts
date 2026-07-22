import type { Database, Json } from "../types/supabase";
import type { DashboardClient } from "./supabase";

/**
 * Every PostgREST read this app performs lives in this module — nothing else
 * touches the network. The app runs as an authenticated teacher with the
 * publishable key; each function's comment cites the shipped RLS policy /
 * grant that admits its rows (RLS is the security boundary — spec
 * docs/specs/449-t4-dashboard.md §4, query→policy map Q1–Q10).
 *
 * Migration line citations:
 *   0006 = supabase/migrations/0006_teacher_dashboard_star_schema.sql
 *   0007 = supabase/migrations/0007_dashboard_grants.sql
 *
 * Deliberately absent: mv_student_day / mv_material_key (service-role only,
 * 0006 L667-670 — a client select is `permission denied` by design), the
 * legacy `sessions` / `session_phrases` push tables (the dashboard reads
 * facts, 0006 header L11-13), and any INSERT/UPDATE/DELETE: this app is a
 * read-only surface.
 */

export type ProfileRow = Database["public"]["Tables"]["profiles"]["Row"];
export type ClassroomRow = Pick<
  Database["public"]["Tables"]["classrooms"]["Row"],
  "id" | "name" | "created_at"
>;
export type RosterHeatRow = Database["public"]["Views"]["v_roster_heat"]["Row"];
export type IntegrityRow =
  Database["public"]["Views"]["v_session_integrity"]["Row"];
export type FactSessionRow =
  Database["public"]["Tables"]["fact_session"]["Row"];
export type ToolEventRow =
  Database["public"]["Tables"]["fact_tool_event"]["Row"];

export interface RosterEntry {
  student_id: string;
  display_name: string | null;
}

export interface ExerciseWithMaterial {
  exercise_id: string;
  logged_at: string;
  tonic: number;
  difficulty: number;
  accuracy: number | null;
  session_id: string | null;
  dim_material: {
    material_id: string;
    label: string;
    source: string;
    kind: string;
    spec_hash: string | null;
  } | null;
}

export interface LateSyncSessionRow {
  session_id: string;
  student_id: string;
  started_at: string;
  created_at: string;
}

export interface PhraseSliceRow {
  session_id: string;
  phrase_index: number;
  start_secs: number;
  end_secs: number;
  note_count: number | null;
}

export interface ToolEventSliceRow {
  session_id: string;
  at_secs: number;
  kind: string;
  params: Json;
}

function unwrap<T>(result: {
  data: T | null;
  error: { message: string } | null;
}): T {
  if (result.error) {
    throw new Error(result.error.message);
  }
  if (result.data === null) {
    throw new Error("no data returned");
  }
  return result.data;
}

/**
 * Q1 — the signed-in user's own profile (the teacher gate reads `role`).
 * Policy: profiles_select_own (0001) — self only. Grant: 0007 L45.
 */
export async function getOwnProfile(
  client: DashboardClient,
  userId: string,
): Promise<ProfileRow | null> {
  const { data, error } = await client
    .from("profiles")
    .select("id, role, display_name, age_tier, created_at")
    .eq("id", userId)
    .maybeSingle();
  if (error) throw new Error(error.message);
  return data;
}

/**
 * Q2 — the teacher's classrooms (the roster containers).
 * Policy: classrooms_select_teacher (0006 L65-66) — owner only; a teacher
 * can never list another teacher's classrooms. Grant: 0006 L56.
 */
export async function listClassrooms(
  client: DashboardClient,
): Promise<ClassroomRow[]> {
  const res = await client
    .from("classrooms")
    .select("id, name, created_at")
    .order("created_at", { ascending: true });
  return unwrap(res);
}

/**
 * Q3 — the ACTIVE roster of one classroom, with display names.
 * Policies: enrollments_select_teacher (0006 L113-118) — own classroom only;
 * profiles_select_enrolled_teacher (0006 L240-247) — display_name is only
 * readable while the enrollment is active, so revocation blanks the name on
 * the next query too. Grants: 0006 L106, 0007 L45.
 * The status filter mirrors (not replaces) RLS: invited/revoked students'
 * facts are invisible regardless (every fact policy re-checks
 * status='active').
 */
export async function listRoster(
  client: DashboardClient,
  classroomId: string,
): Promise<RosterEntry[]> {
  const res = await client
    .from("enrollments")
    .select("student_id, profiles ( display_name )")
    .eq("classroom_id", classroomId)
    .eq("status", "active");
  const rows = unwrap(res) as unknown as Array<{
    student_id: string;
    profiles: { display_name: string | null } | null;
  }>;
  return rows.map((r) => ({
    student_id: r.student_id,
    display_name: r.profiles?.display_name ?? null,
  }));
}

/**
 * Q4 — roster heat rows for the window (spec AC2).
 * View: v_roster_heat, security_invoker (0006 L703-719) — the querying
 * teacher's own policies gate every underlying row:
 * fact_session_select_teacher (0006 L498-505, active-enrollment join),
 * enrollments_select_teacher, and the profiles policy/grant from Q3.
 * Grant: 0006 L719.
 */
export async function getRosterHeat(
  client: DashboardClient,
  classroomId: string,
  sinceDateKey: string,
): Promise<RosterHeatRow[]> {
  const res = await client
    .from("v_roster_heat")
    .select("*")
    .eq("classroom_id", classroomId)
    .gte("date_key", sinceDateKey);
  return unwrap(res);
}

/**
 * Q5 — session arrival times for the F9 late-sync annotation ("synced
 * Jun 3, played May 1-30" — shown, not punished; datamodel §5 F9).
 * `created_at` is server arrival (0006 L470 comment: "the F9 late-sync
 * signal"). Policy: fact_session_select_teacher (0006 L498-505).
 * Grant: 0006 L479.
 */
export async function getLateSyncSessions(
  client: DashboardClient,
  studentIds: string[],
  sinceIso: string,
): Promise<LateSyncSessionRow[]> {
  if (studentIds.length === 0) return [];
  const res = await client
    .from("fact_session")
    .select("session_id, student_id, started_at, created_at")
    .in("student_id", studentIds)
    .gte("started_at", sinceIso);
  return unwrap(res);
}

/**
 * Q6 — one student's recent sessions for the drill-down timeline (played vs
 * wall, notes, silence, fingerprint recap facts — spec AC5).
 * Policy: fact_session_select_teacher (0006 L498-505). Grant: 0006 L479.
 */
export async function getStudentSessions(
  client: DashboardClient,
  studentId: string,
): Promise<FactSessionRow[]> {
  const res = await client
    .from("fact_session")
    .select("*")
    .eq("student_id", studentId)
    .order("started_at", { ascending: false })
    .limit(50);
  return unwrap(res);
}

/**
 * Q7 — one student's exercise facts joined to material (the coverage
 * matrix: spec_hash × tonic — the RV "did she row the cell through the
 * keys" question; spec AC6). Policies: fact_exercise_select_teacher (0006
 * L582-589); dim_material_select_signed_in (0006 L446-447 — shared
 * dimension, no per-student data). Grants: 0006 L567, L434.
 */
export async function getStudentExercises(
  client: DashboardClient,
  studentId: string,
): Promise<ExerciseWithMaterial[]> {
  const res = await client
    .from("fact_exercise")
    .select(
      "exercise_id, logged_at, tonic, difficulty, accuracy, session_id, " +
        "dim_material ( material_id, label, source, kind, spec_hash )",
    )
    .eq("student_id", studentId)
    .order("logged_at", { ascending: false })
    .limit(1000);
  return unwrap(res) as unknown as ExerciseWithMaterial[];
}

/**
 * Q8 — one student's tool events for the usage summary (pocket / band /
 * narration; spec AC7). Policy: fact_tool_event_select_teacher (0006
 * L619-626). Grant: 0006 L605.
 */
export async function getStudentToolEvents(
  client: DashboardClient,
  studentId: string,
): Promise<ToolEventRow[]> {
  const res = await client
    .from("fact_tool_event")
    .select("*")
    .eq("student_id", studentId)
    .limit(2000);
  return unwrap(res);
}

/**
 * Q9 — the integrity panel's evidence rows (spec AC8/AC9).
 * View: v_session_integrity, security_invoker (0006 L725-744) — thresholds
 * live in the view (datamodel §4d verbatim); the teacher's own
 * fact_session/fact_exercise policies gate the rows. The view has no
 * classroom column, so the caller filters to the selected roster's
 * student_ids client-side. Grant: 0006 L744.
 */
export async function getIntegrityRows(
  client: DashboardClient,
): Promise<IntegrityRow[]> {
  const res = await client
    .from("v_session_integrity")
    .select("*")
    .order("started_at", { ascending: false })
    .limit(100);
  return unwrap(res);
}

/**
 * Q10 — F8 evidence for flagged sessions: tool events × phrases, range-join
 * computed client-side (datamodel §5 F8: "tool-on spans with near-zero
 * phrase note_count inside them"). Policies: fact_tool_event_select_teacher
 * (0006 L619-626); fact_phrase_select_teacher (0006 L538-545). Grants:
 * 0006 L605, L520.
 */
export async function getSessionEvidence(
  client: DashboardClient,
  sessionIds: string[],
): Promise<{ events: ToolEventSliceRow[]; phrases: PhraseSliceRow[] }> {
  if (sessionIds.length === 0) return { events: [], phrases: [] };
  const eventsRes = await client
    .from("fact_tool_event")
    .select("session_id, at_secs, kind, params")
    .in("session_id", sessionIds);
  const phrasesRes = await client
    .from("fact_phrase")
    .select("session_id, phrase_index, start_secs, end_secs, note_count")
    .in("session_id", sessionIds);
  return { events: unwrap(eventsRes), phrases: unwrap(phrasesRes) };
}
