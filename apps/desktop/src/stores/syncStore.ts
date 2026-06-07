import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { supabase } from "../lib/supabase";
import type { Json } from "../types/supabase";
import type { SessionSummaryDto, StoredSessionDto } from "../types/brain";

/**
 * Cloud sync for completed practice sessions (Phase 3, Teacher Dashboard
 * track). The Rust store is the source of truth: we read persisted sessions
 * over IPC and push them up to Supabase. This is plumbing, not business logic
 * — every recap is already computed in the Rust core.
 *
 * Sync is one-directional (local → cloud) and idempotent: rows are upserted
 * keyed on the local session id, and we remember which ids we've already
 * pushed (per user) so a re-sync doesn't re-fetch every detail. Only the
 * session-level recap is synced today; per-phrase rows wait on phrase data
 * being exposed over IPC for stored sessions (see brain.ts).
 */
export type SyncStatus = "idle" | "syncing" | "synced" | "error";

export interface SyncState {
  status: SyncStatus;
  lastSyncedAt: number | null;
  /** Sessions pushed in the most recent run. */
  syncedThisRun: number;
  error: string | null;

  /**
   * Push any not-yet-synced local sessions for `userId`. No-op (returns to
   * idle) when called with a falsy userId, so callers can wire it to an auth
   * effect without guarding.
   */
  syncAll: (userId: string | null | undefined) => Promise<void>;
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

export const useSyncStore = create<SyncState>((set) => ({
  status: "idle",
  lastSyncedAt: null,
  syncedThisRun: 0,
  error: null,

  syncAll: async (userId) => {
    if (!userId) {
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
            // ToneDescriptor is a flat numeric record — safe as jsonb. Sourced
            // from the unified fingerprint; the DB column keeps its name.
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

  reset: () =>
    set({ status: "idle", lastSyncedAt: null, syncedThisRun: 0, error: null }),
}));
