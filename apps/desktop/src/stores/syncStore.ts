import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { supabase } from "../lib/supabase";
import type { Database, Json } from "../types/supabase";
import type {
  ExerciseFactRow,
  PhraseFactDto,
  ScoreRefDto,
  SessionFactDto,
  SessionProjectionDto,
  SessionSummaryDto,
  StoredSessionDto,
  TasteProfile,
  ToolEventFactDto,
} from "../types/brain";

/**
 * Cloud sync for completed practice sessions (Phase 3, Teacher Dashboard
 * track) and — behind its own independent opt-in — the personalization taste
 * profile (Phase 4). The Rust store is the source of truth: we read persisted
 * data over IPC and push it up to Supabase. This is plumbing, not business
 * logic — every recap/profile is already computed/captured in the Rust core.
 *
 * Sync is one-directional (local → cloud) and idempotent: session rows are
 * upserted keyed on the local session id, and we remember which ids we've
 * already pushed (per user) so a re-sync doesn't re-fetch every detail.
 *
 * Independent switches: session sync and taste-profile sync are separate
 * opt-ins on purpose (personalization spine §Privacy — "the same
 * independent-switches model as session data"). Turning one on never implies
 * the other, and neither is entangled with teacher linking.
 *
 * Every switch here is a *networked feature*: it is opt-in, off by default,
 * and enumerated in `docs/architecture/offline-first-and-network-transparency.md`
 * and surfaced (with plain-language disclosure of what leaves the device) in
 * `ConnectionsPrivacy.tsx`. The core practice loop never calls into this store.
 */
export type SyncStatus = "idle" | "syncing" | "synced" | "error";

// ---------------------------------------------------------------------------
// #449 T2: the teacher-dashboard projection (device → cloud, doc §2 P1–P4).
//
// Everything below writes ONLY the star-schema tables of migration
// 0006_teacher_dashboard_star_schema.sql, column-for-column (the typed
// client enforces it against types/supabase.ts, which mirrors the
// migration). Payload builders are exported for tests; the type-level pins
// directly below are the structural privacy contract.
// ---------------------------------------------------------------------------

type Tables = Database["public"]["Tables"];
type FactSessionInsert = Tables["fact_session"]["Insert"];
type FactPhraseInsert = Tables["fact_phrase"]["Insert"];
type FactExerciseInsert = Tables["fact_exercise"]["Insert"];
type FactToolEventInsert = Tables["fact_tool_event"]["Insert"];
type DimMaterialInsert = Tables["dim_material"]["Insert"];

/**
 * STRUCTURAL PRIVACY PIN (doc §2, spec AC4). `spec_json` and `seed` NEVER
 * cross the wire (P3: "replayability is a device concern"); neither does
 * the full phrase payload (P2: "no onsets vector, no pitch curves"). The
 * assertion is on the TYPES: if any payload type (device-side DTO or cloud
 * Insert row) ever gains one of these keys, `Extract<...>` stops being
 * `never` and this file stops compiling. Do not "fix" the pin — fix the
 * type that grew a forbidden field.
 */
type ForbiddenPayloadKey = "spec_json" | "seed" | "phrase_json" | "onsets_secs";
type AssertNever<T extends never> = T;
export type _DashboardExercisePayloadPin = AssertNever<
  Extract<
    keyof ExerciseFactRow | keyof FactExerciseInsert | keyof DimMaterialInsert,
    ForbiddenPayloadKey
  >
>;
export type _DashboardPhrasePayloadPin = AssertNever<
  Extract<keyof PhraseFactDto | keyof FactPhraseInsert, ForbiddenPayloadKey>
>;
export type _DashboardSessionPayloadPin = AssertNever<
  Extract<keyof SessionFactDto | keyof FactSessionInsert, ForbiddenPayloadKey>
>;

/**
 * P1: one `fact_session` row — column-for-column against 0006's
 * `create table public.fact_session` (lines 452–472; a drifted name here is
 * a silent 400 on the real stack). Idempotent on the 0006 L472 unique
 * `(student_id, device_session_id)`.
 */
export function buildFactSessionRow(
  userId: string,
  s: SessionFactDto,
  scoreMaterialId: string | null,
): FactSessionInsert {
  return {
    student_id: userId,
    device_session_id: s.id,
    started_at: s.started_at,
    ended_at: s.ended_at,
    duration_secs: s.duration_secs,
    played_secs: s.played_secs,
    note_count: s.note_count,
    silence_ratio: s.silence_ratio,
    phrase_count: s.phrase_count,
    instrument: s.instrument,
    practice_mode: s.practice_mode,
    score_material_id: scoreMaterialId,
    fingerprint: (s.fingerprint ?? null) as Json,
    app_version: s.app_version,
  };
}

/**
 * P2: thin `fact_phrase` rows, keyed by the CLOUD session uuid —
 * column-for-column against 0006's `create table public.fact_phrase`
 * (lines 508–518, PK `(session_id, phrase_index)` at L517).
 */
export function buildFactPhraseRows(
  cloudSessionId: string,
  phrases: PhraseFactDto[],
): FactPhraseInsert[] {
  return phrases.map((p) => ({
    session_id: cloudSessionId,
    phrase_index: p.phrase_index,
    start_secs: p.start_secs,
    end_secs: p.end_secs,
    note_count: p.note_count,
    stability: p.stability,
    tone: (p.tone ?? null) as Json,
    key_name: p.key_name,
  }));
}

/**
 * P4: `fact_tool_event` rows — column-for-column against 0006's
 * `create table public.fact_tool_event` (lines 592–602, unique
 * `(session_id, device_event_id)` at L601). `params` is the T1
 * vocabulary — ids-and-numbers-only, no content (doc §1a).
 *
 * SEMANTICS (#470, option (a) — this is the projection site): a
 * `narration_used {"kind":"recap"}` event means **the recap's headline text
 * was LLM-authored**. The parser requires a non-empty `overall_assessment`
 * (a wrong-keys or blank-headline response is a parse failure that serves
 * the on-device fallback and journals nothing), so a dashboard surface may
 * phrase this event as "AI narration used". The secondary list fields stay
 * individually forgiving — only the shown headline is guaranteed
 * LLM-authored, not every list item.
 */
export function buildFactToolEventRows(
  cloudSessionId: string,
  userId: string,
  events: ToolEventFactDto[],
): FactToolEventInsert[] {
  return events.map((e) => ({
    session_id: cloudSessionId,
    student_id: userId,
    device_event_id: e.device_event_id,
    at_secs: e.at_secs,
    kind: e.kind,
    params: parseParams(e.params_json),
  }));
}

/**
 * Defensive parse of the journal's params_json; `{}` over garbage — and
 * over any non-object JSON. The second clause is load-bearing:
 * `JSON.parse("null")` SUCCEEDS and returns `null`, but
 * `fact_tool_event.params` is NOT NULL (0006 L599) — pushing it through
 * would 400 that session on EVERY run, permanently wedging it out of the
 * synced set. Arrays/numbers/strings are coerced for the same reason: the
 * T1 vocabulary is an object, and anything else is corruption, not data.
 */
function parseParams(paramsJson: string): Json {
  try {
    const parsed: unknown = JSON.parse(paramsJson);
    return typeof parsed === "object" &&
      parsed !== null &&
      !Array.isArray(parsed)
      ? (parsed as Json)
      : {};
  } catch {
    return {};
  }
}

/**
 * P3 (dimension half): distinct `dim_material` rows for the batch's spec
 * hashes — columns per 0006's `create table public.dim_material` (lines
 * 419–432; `spec_hash` unique at L421, the `dim_material_keyed` CHECK wants
 * `kind='cell'` ⇒ `spec_hash` present). Grain rule (doc §3): the 12 keys of
 * one cell are ONE material row — tonic lives on the fact, so the dimension
 * keys on `spec_hash` only.
 */
export function buildDimMaterialRows(
  facts: ExerciseFactRow[],
): DimMaterialInsert[] {
  const byHash = new Map<string, DimMaterialInsert>();
  for (const f of facts) {
    if (!byHash.has(f.spec_hash)) {
      byHash.set(f.spec_hash, {
        spec_hash: f.spec_hash,
        label: f.label,
        source: f.source,
        kind: "cell",
      });
    }
  }
  return [...byHash.values()];
}

/**
 * P3 (fact half): `fact_exercise` rows for every fact whose material row
 * resolved — column-for-column against 0006's `create table
 * public.fact_exercise` (lines 551–563, unique `(student_id,
 * device_log_id)` at L562; NO spec_json/seed columns exist there, by
 * design). Rows with an unresolved hash are SKIPPED (and the watermark
 * must not advance past them — see `syncDashboard`).
 */
export function buildFactExerciseRows(
  userId: string,
  facts: ExerciseFactRow[],
  materialIdByHash: Map<string, string>,
): FactExerciseInsert[] {
  const rows: FactExerciseInsert[] = [];
  for (const f of facts) {
    const materialId = materialIdByHash.get(f.spec_hash);
    if (!materialId) continue;
    rows.push({
      student_id: userId,
      device_log_id: f.id,
      logged_at: f.logged_at,
      material_id: materialId,
      tonic: f.tonic,
      difficulty: f.difficulty,
      accuracy: f.accuracy,
      // session_id deliberately omitted → NULL: the local exercise_log has
      // no session linkage yet ("session linkage when known", 0006).
    });
  }
  return rows;
}

export interface SyncState {
  status: SyncStatus;
  lastSyncedAt: number | null;
  /** Sessions pushed in the most recent run. */
  syncedThisRun: number;
  error: string | null;

  /** Independent status for the taste-profile sync switch. */
  tasteProfileStatus: SyncStatus;
  tasteProfileError: string | null;

  /** #449 T2: independent status for the dashboard projection. */
  dashboardStatus: SyncStatus;
  dashboardError: string | null;
  /** Sessions projected (P1–P2–P4) in the most recent dashboard run. */
  dashboardSyncedThisRun: number;

  /**
   * #449 T2: push the dashboard projection (doc §2 P1–P4) — session facts,
   * thin phrase rows, exercise rows, tool events — to the star schema.
   *
   * Gating (spec §Gating matrix): runs ONLY when signed in AND cloud sync
   * is on AND the SEPARATE `dashboardSyncEnabled` opt-in is on. With the
   * dashboard toggle off, the legacy session push (`syncAll`) keeps its
   * existing behavior and nothing else leaves the device (honest absence).
   *
   * P5 note (doc §2, re-read): key_mastery rides the EXISTING learner-model
   * push (`syncLearnerModel`) — no new work here, its behavior unchanged.
   *
   * Incremental by watermark, idempotent on device ids, calm on failure
   * (offline is normal life; the next trigger retries). Never called on the
   * audio path — only from the existing sync trigger after sessions close.
   */
  syncDashboard: (
    userId: string | null | undefined,
    cloudSyncOptedIn: boolean,
    dashboardOptedIn: boolean,
  ) => Promise<void>;

  /**
   * Push any not-yet-synced local sessions for `userId`, but only when the
   * user has opted into session sync (`optedIn`). No-op (returns to idle) when
   * called with a falsy userId or when optedIn is false.
   */
  syncAll: (
    userId: string | null | undefined,
    optedIn: boolean,
  ) => Promise<void>;

  /**
   * Push the local taste profile for `userId` to Supabase — but only when the
   * user has opted into profile sync (`optedIn`). This is its OWN switch,
   * deliberately decoupled from session sync and from linking: a user can sync
   * sessions without syncing preferences, and vice versa. No-op when `userId`
   * is falsy or `optedIn` is false.
   */
  /**
   * Push the local Learner Model (collection, mastery, difficulty) to the
   * cloud (#252 F2). Rides the SAME opt-in as taste-profile sync — progress
   * data, one switch. Push-only: local is authoritative.
   */
  syncLearnerModel: (userId: string | null, optedIn: boolean) => Promise<void>;
  syncTasteProfile: (
    userId: string | null | undefined,
    optedIn: boolean,
  ) => Promise<void>;

  reset: () => void;
}

/** localStorage key for the set of session ids already pushed for a user. */
function syncedKey(userId: string): string {
  return `ai-music-companion:synced-sessions:${userId}`;
}

function loadSyncedIds(userId: string): Set<string> {
  try {
    const raw = localStorage.getItem(syncedKey(userId));
    if (!raw) return new Set();
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((x): x is string => typeof x === "string"))
      : new Set();
  } catch {
    return new Set();
  }
}

function saveSyncedIds(userId: string, ids: Set<string>): void {
  try {
    localStorage.setItem(syncedKey(userId), JSON.stringify([...ids]));
  } catch {
    // localStorage unavailable — sync still works, it just won't skip
    // already-pushed sessions next time. Harmless (upsert is idempotent).
  }
}

// ---------------------------------------------------------------------------
// #449 T2 watermarks — the syncedKey idiom, per surface (spec §Watermarks).
// Sessions are immutable after close, so a per-user SET of projected device
// session ids is the P1/P2/P4 watermark; P3 keeps a numeric high-water mark
// over exercise_log.id. Losing either is harmless: every cloud write is
// idempotent on device ids.
// ---------------------------------------------------------------------------

function dashboardSyncedKey(userId: string): string {
  return `ai-music-companion:dashboard-synced-sessions:${userId}`;
}

function loadDashboardSyncedIds(userId: string): Set<string> {
  try {
    const raw = localStorage.getItem(dashboardSyncedKey(userId));
    if (!raw) return new Set();
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((x): x is string => typeof x === "string"))
      : new Set();
  } catch {
    return new Set();
  }
}

function saveDashboardSyncedIds(userId: string, ids: Set<string>): void {
  try {
    localStorage.setItem(dashboardSyncedKey(userId), JSON.stringify([...ids]));
  } catch {
    // Harmless — re-pushes are absorbed by the device-id conflict keys.
  }
}

function exerciseWatermarkKey(userId: string): string {
  return `ai-music-companion:dashboard-exercise-watermark:${userId}`;
}

function loadExerciseWatermark(userId: string): number {
  try {
    const raw = localStorage.getItem(exerciseWatermarkKey(userId));
    const n = raw == null ? 0 : Number(raw);
    return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
  } catch {
    return 0;
  }
}

function saveExerciseWatermark(userId: string, id: number): void {
  try {
    localStorage.setItem(exerciseWatermarkKey(userId), String(id));
  } catch {
    // Harmless — re-pushes are absorbed by (student_id, device_log_id).
  }
}

/**
 * Resolve (or lazily create) the `dim_material` row for a practised score
 * (P1 "score_id→title" → kind 'score'). `dim_material` has no unique key on
 * `score_id` (0006 — only `spec_hash` is unique), so this is select-first,
 * insert-if-absent rather than an upsert.
 */
async function ensureScoreMaterialId(score: ScoreRefDto): Promise<string> {
  const found = await supabase
    .from("dim_material")
    .select("material_id")
    .eq("score_id", score.score_id)
    .eq("kind", "score")
    .limit(1)
    .maybeSingle();
  if (found.error) throw new Error(found.error.message);
  if (found.data) return found.data.material_id;
  const inserted = await supabase
    .from("dim_material")
    .insert({
      score_id: score.score_id,
      label: score.title,
      source: "score_practice",
      kind: "score",
    })
    .select("material_id")
    .single();
  if (inserted.error) throw new Error(inserted.error.message);
  return inserted.data.material_id;
}

export const useSyncStore = create<SyncState>((set, get) => ({
  status: "idle",
  lastSyncedAt: null,
  syncedThisRun: 0,
  error: null,
  tasteProfileStatus: "idle",
  tasteProfileError: null,
  dashboardStatus: "idle",
  dashboardError: null,
  dashboardSyncedThisRun: 0,

  syncDashboard: async (userId, cloudSyncOptedIn, dashboardOptedIn) => {
    // The gate (spec AC1): signed in AND cloud sync on AND the separate
    // dashboard opt-in on — otherwise nothing is read and nothing leaves.
    if (!userId || !cloudSyncOptedIn || !dashboardOptedIn) {
      set({ dashboardStatus: "idle", dashboardError: null });
      return;
    }
    set({
      dashboardStatus: "syncing",
      dashboardError: null,
      dashboardSyncedThisRun: 0,
    });
    try {
      // ── P1 + P2 + P4: per closed session not yet projected ──────────────
      const summaries = await invoke<SessionSummaryDto[]>(
        "get_session_history",
        { instrument_filter: null, start_date: null, end_date: null },
      );
      const projected = loadDashboardSyncedIds(userId);
      const pending = summaries.filter((s) => !projected.has(s.id));
      let sessionsThisRun = 0;

      for (const summary of pending) {
        const proj = await invoke<SessionProjectionDto>(
          "get_session_projection",
          { session_id: summary.id },
        );

        const scoreMaterialId = proj.session.score
          ? await ensureScoreMaterialId(proj.session.score)
          : null;

        // fact_session: idempotent upsert on (student, device session);
        // the returned cloud uuid keys the child rows.
        const sessionRes = await supabase
          .from("fact_session")
          .upsert(buildFactSessionRow(userId, proj.session, scoreMaterialId), {
            onConflict: "student_id,device_session_id",
          })
          .select("session_id")
          .single();
        if (sessionRes.error) throw new Error(sessionRes.error.message);
        const cloudSessionId = sessionRes.data.session_id;

        // fact_phrase / fact_tool_event are insert-only for clients (0006):
        // ignoreDuplicates makes a re-push a no-op, not a permission error.
        if (proj.phrases.length > 0) {
          const { error } = await supabase
            .from("fact_phrase")
            .upsert(buildFactPhraseRows(cloudSessionId, proj.phrases), {
              onConflict: "session_id,phrase_index",
              ignoreDuplicates: true,
            });
          if (error) throw new Error(error.message);
        }
        if (proj.events.length > 0) {
          const { error } = await supabase
            .from("fact_tool_event")
            .upsert(
              buildFactToolEventRows(cloudSessionId, userId, proj.events),
              {
                onConflict: "session_id,device_event_id",
                ignoreDuplicates: true,
              },
            );
          if (error) throw new Error(error.message);
        }

        // Only a fully-landed session is remembered — a failure above threw
        // and leaves it pending for the next run (idempotent re-push).
        projected.add(summary.id);
        saveDashboardSyncedIds(userId, projected);
        sessionsThisRun += 1;
      }

      // ── P3: exercises past the numeric watermark ────────────────────────
      const watermark = loadExerciseWatermark(userId);
      const facts = await invoke<ExerciseFactRow[]>("list_exercise_facts", {
        after_id: watermark,
      });
      if (facts.length > 0) {
        const dimRes = await supabase
          .from("dim_material")
          .upsert(buildDimMaterialRows(facts), {
            onConflict: "spec_hash",
            ignoreDuplicates: true,
          });
        if (dimRes.error) throw new Error(dimRes.error.message);

        // ignoreDuplicates returns only NEW rows, so resolve every hash by
        // reading the dimension back (shared, select-granted — 0006).
        const hashes = [...new Set(facts.map((f) => f.spec_hash))];
        const matRes = await supabase
          .from("dim_material")
          .select("material_id,spec_hash")
          .in("spec_hash", hashes);
        if (matRes.error) throw new Error(matRes.error.message);
        const materialIdByHash = new Map<string, string>();
        for (const m of matRes.data) {
          if (m.spec_hash != null) {
            materialIdByHash.set(m.spec_hash, m.material_id);
          }
        }

        const rows = buildFactExerciseRows(userId, facts, materialIdByHash);
        if (rows.length > 0) {
          const { error } = await supabase.from("fact_exercise").upsert(rows, {
            onConflict: "student_id,device_log_id",
            ignoreDuplicates: true,
          });
          if (error) throw new Error(error.message);
        }

        // Advance the watermark, but never past an unresolved row — those
        // must resurface next run (spec §Edge cases).
        const unresolved = facts.filter(
          (f) => !materialIdByHash.has(f.spec_hash),
        );
        const next =
          unresolved.length > 0
            ? Math.min(...unresolved.map((f) => f.id)) - 1
            : Math.max(...facts.map((f) => f.id));
        if (next > watermark) {
          saveExerciseWatermark(userId, next);
        }
      }

      set({
        dashboardStatus: "synced",
        dashboardError: null,
        dashboardSyncedThisRun: sessionsThisRun,
      });
    } catch (err: unknown) {
      // Calm failure: offline is normal life. Nothing partial was marked
      // done, so the next trigger simply retries.
      set({ dashboardStatus: "error", dashboardError: String(err) });
    }
  },

  syncAll: async (userId, optedIn) => {
    if (!userId || !optedIn) {
      set({ status: "idle", error: null });
      return;
    }
    set({ status: "syncing", error: null, syncedThisRun: 0 });
    try {
      const summaries = await invoke<SessionSummaryDto[]>(
        "get_session_history",
        { instrument_filter: null, start_date: null, end_date: null },
      );

      const alreadySynced = loadSyncedIds(userId);
      const pending = summaries.filter((s) => !alreadySynced.has(s.id));

      if (pending.length === 0) {
        set({ status: "synced", lastSyncedAt: Date.now(), syncedThisRun: 0 });
        return;
      }

      const rows = await Promise.all(
        pending.map(async (summary) => {
          const detail = await invoke<StoredSessionDto>("get_session_detail", {
            session_id: summary.id,
          });
          const { recap } = detail;
          return {
            id: detail.id,
            student_id: userId,
            instrument: recap.instrument,
            started_at: detail.started_at,
            ended_at: detail.ended_at,
            duration_secs: recap.duration_secs,
            phrase_count: recap.phrase_count,
            overall_assessment: recap.overall_assessment,
            // The full unified MusicalFingerprint (tone, key, intonation,
            // groove) as forward-compatible JSONB — the contract the
            // personalization layer reads.
            fingerprint: (recap.fingerprint ?? null) as Json,
            // ToneDescriptor is a flat numeric record — safe as jsonb. Still
            // projected for backward compatibility with the existing
            // tone-only readers; the DB column keeps its name.
            session_tone: (recap.fingerprint?.tone ?? null) as Json,
          };
        }),
      );

      const { error } = await supabase
        .from("sessions")
        .upsert(rows, { onConflict: "id" });
      if (error) throw new Error(error.message);

      for (const r of rows) alreadySynced.add(r.id);
      saveSyncedIds(userId, alreadySynced);

      set({
        status: "synced",
        lastSyncedAt: Date.now(),
        syncedThisRun: rows.length,
      });
    } catch (err: unknown) {
      set({ status: "error", error: String(err) });
    }
  },

  syncLearnerModel: async (userId, optedIn) => {
    if (!userId || !optedIn) {
      return;
    }
    try {
      const blob = await invoke<Json | null>("get_learner_model_blob");
      if (blob == null) {
        return; // cold start — nothing to push yet
      }
      const { error } = await supabase.from("learner_model").upsert(
        {
          user_id: userId,
          model: blob,
          updated_at: new Date().toISOString(),
        },
        { onConflict: "user_id" },
      );
      if (error) throw new Error(error.message);
    } catch (err: unknown) {
      // Best-effort: progress sync must never disrupt the app; the local
      // model stays authoritative and the next sync retries.
      console.error("learner model sync failed:", err);
    }
  },

  syncTasteProfile: async (userId, optedIn) => {
    // Independent switch: do nothing unless signed in AND opted into profile
    // sync. Never implied by session sync.
    if (!userId || !optedIn) {
      set({ tasteProfileStatus: "idle", tasteProfileError: null });
      return;
    }
    set({ tasteProfileStatus: "syncing", tasteProfileError: null });
    try {
      const profile = await invoke<TasteProfile>("get_taste_profile");
      const { error } = await supabase.from("taste_profile").upsert(
        {
          user_id: userId,
          genres: profile.genres as Json,
          artists: profile.artists as Json,
          goals: profile.goals as Json,
          experience: profile.experience,
          is_under_13: profile.is_under_13,
          updated_at: new Date().toISOString(),
        },
        { onConflict: "user_id" },
      );
      if (error) throw new Error(error.message);
      set({ tasteProfileStatus: "synced", tasteProfileError: null });
      // Progress data rides the same opt-in: push the Learner Model too.
      void get().syncLearnerModel(userId, optedIn);
    } catch (err: unknown) {
      set({ tasteProfileStatus: "error", tasteProfileError: String(err) });
    }
  },

  reset: () =>
    set({
      status: "idle",
      lastSyncedAt: null,
      syncedThisRun: 0,
      error: null,
      tasteProfileStatus: "idle",
      tasteProfileError: null,
      dashboardStatus: "idle",
      dashboardError: null,
      dashboardSyncedThisRun: 0,
    }),
}));
