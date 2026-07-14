import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { supabase } from "../lib/supabase";
import type { Json } from "../types/supabase";
import type {
  SessionSummaryDto,
  StoredSessionDto,
  TasteProfile,
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

export interface SyncState {
  status: SyncStatus;
  lastSyncedAt: number | null;
  /** Sessions pushed in the most recent run. */
  syncedThisRun: number;
  error: string | null;

  /** Independent status for the taste-profile sync switch. */
  tasteProfileStatus: SyncStatus;
  tasteProfileError: string | null;

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

export const useSyncStore = create<SyncState>((set, get) => ({
  status: "idle",
  lastSyncedAt: null,
  syncedThisRun: 0,
  error: null,
  tasteProfileStatus: "idle",
  tasteProfileError: null,

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
    }),
}));
