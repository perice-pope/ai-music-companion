import { create } from "zustand";

/**
 * View state — three surfaces, no router dependency (spec §8): the roster
 * heat (landing), a per-student drill-down, and the integrity panel.
 * Classroom selection lives here too so every surface is per-classroom
 * (the product is priced per-classroom/year — founder decision).
 */
export type View =
  | { kind: "roster" }
  | { kind: "student"; studentId: string; displayName: string | null }
  | { kind: "integrity" };

export interface NavState {
  classroomId: string | null;
  view: View;
  selectClassroom: (id: string) => void;
  openStudent: (studentId: string, displayName: string | null) => void;
  showRoster: () => void;
  showIntegrity: () => void;
}

export const useNavStore = create<NavState>((set) => ({
  classroomId: null,
  view: { kind: "roster" },
  selectClassroom: (id) => set({ classroomId: id, view: { kind: "roster" } }),
  openStudent: (studentId, displayName) =>
    set({ view: { kind: "student", studentId, displayName } }),
  showRoster: () => set({ view: { kind: "roster" } }),
  showIntegrity: () => set({ view: { kind: "integrity" } }),
}));
