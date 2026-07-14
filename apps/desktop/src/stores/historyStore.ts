import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  SessionSummaryDto,
  StoredSessionDto,
  PracticeStatsDto,
} from "../types/brain";

export interface HistoryState {
  sessions: SessionSummaryDto[];
  stats: PracticeStatsDto | null;
  selectedSessionId: string | null;
  selectedSessionDetail: StoredSessionDto | null;
  instrumentFilter: string | null;
  dateRangeFilter: { start?: Date; end?: Date } | null;
  isLoading: boolean;
  error: string | null;

  loadHistory: () => Promise<void>;
  loadStats: () => Promise<void>;
  loadSessionDetail: (id: string) => Promise<void>;
  setInstrumentFilter: (instrument: string | null) => void;
  setDateRangeFilter: (range: { start?: Date; end?: Date } | null) => void;
  clearError: () => void;
}

export const useHistoryStore = create<HistoryState>((set, get) => ({
  sessions: [],
  stats: null,
  selectedSessionId: null,
  selectedSessionDetail: null,
  instrumentFilter: null,
  dateRangeFilter: null,
  isLoading: false,
  error: null,

  loadHistory: async () => {
    set({ isLoading: true, error: null });
    try {
      const { instrumentFilter, dateRangeFilter } = get();
      const sessions = await invoke<SessionSummaryDto[]>(
        "get_session_history",
        {
          instrument_filter: instrumentFilter,
          start_date: dateRangeFilter?.start?.toISOString() || null,
          end_date: dateRangeFilter?.end?.toISOString() || null,
        }
      );
      set({ sessions, isLoading: false });
    } catch (err: unknown) {
      set({
        error: String(err),
        isLoading: false,
      });
    }
  },

  loadStats: async () => {
    try {
      const stats = await invoke<PracticeStatsDto>("get_practice_stats");
      set({ stats });
    } catch (err: unknown) {
      set({
        error: `Failed to load stats: ${String(err)}`,
      });
    }
  },

  loadSessionDetail: async (id: string) => {
    try {
      const selectedSessionDetail = await invoke<StoredSessionDto>(
        "get_session_detail",
        { session_id: id }
      );
      set({ selectedSessionId: id, selectedSessionDetail });
    } catch (err: unknown) {
      set({
        error: `Failed to load session: ${String(err)}`,
      });
    }
  },

  setInstrumentFilter: (instrument: string | null) => {
    set({ instrumentFilter: instrument });
    get().loadHistory();
  },

  setDateRangeFilter: (range: { start?: Date; end?: Date } | null) => {
    set({ dateRangeFilter: range });
    get().loadHistory();
  },

  clearError: () => {
    set({ error: null });
  },
}));
