/**
 * TypeScript mirrors of the Rust serde shapes for Story #14.
 *
 * This file is hand-maintained. When the Rust side changes (new fields,
 * renamed fields, changed casing) update this file in the same commit
 * and add a roundtrip assertion in a Vitest so drift is caught.
 *
 * Keep this file lean: only types that cross the Tauri IPC boundary
 * belong here.
 */

/**
 * Coaching severity. Matches `brain::coaching::CoachingSeverity` which
 * serialises in `snake_case`.
 */
export type CoachingSeverity = "encouragement" | "suggestion" | "focus";

/**
 * Coaching category. Matches `brain::coaching::CoachingCategory`.
 */
export type CoachingCategory =
  | "tone"
  | "intonation"
  | "rhythm"
  | "dynamics"
  | "expression"
  | "technique";

/** A coaching tip returned by the backend. */
export interface CoachingTip {
  text: string;
  severity: CoachingSeverity;
  category: CoachingCategory;
}

/**
 * Pitch stats for a phrase. Mirrors `brain::phrase::PitchStats`.
 * We expose only what the frontend needs today — add fields when used.
 */
export interface PitchStats {
  mean_hz: number;
  min_hz: number;
  max_hz: number;
  range_cents: number;
  pitches: number[];
}

/** Dynamics stats for a phrase. Mirrors `brain::phrase::DynamicsStats`. */
export interface DynamicsStats {
  mean_amplitude: number;
  min_amplitude: number;
  max_amplitude: number;
  dynamic_range: number;
}

/**
 * Summary of a completed musical phrase. Matches
 * `brain::phrase::PhraseSummary`.
 *
 * Only consumed by the frontend for the recap flow; in PR 2 the live
 * session will stream these over the `phrase-detected` event.
 */
export interface PhraseSummary {
  phrase_index: number;
  start_time: number;
  end_time: number;
  duration_secs: number;
  note_count: number;
  pitch_stats: PitchStats;
  dynamics: DynamicsStats;
  stability: number;
}

/**
 * Post-session recap shown on the `recap` screen. Matches
 * `brain::session::SessionRecap`.
 *
 * Important product invariant: the UI renders `strengths` before
 * `areas_to_improve`. The order is encoded by the component, not by
 * sorting this struct's fields.
 */
export interface SessionRecap {
  overall_assessment: string;
  strengths: string[];
  areas_to_improve: string[];
  next_session_suggestions: string[];
  duration_secs: number;
  phrase_count: number;
  instrument: string;
}

/**
 * An instrument the user can choose at session start. Matches
 * `commands::InstrumentInfo` on the Rust side.
 */
export interface InstrumentInfo {
  name: string;
  family: string;
}

/** Payload of the `session-status` Tauri event. */
export interface SessionStatusPayload {
  status: "starting" | "listening" | "ending";
}

/** Payload of the `segment-changed` Tauri event. */
export interface SegmentChangedPayload {
  segment_id: string;
  instrument: string;
  /** RFC 3339 timestamp string. */
  started_at: string;
}
