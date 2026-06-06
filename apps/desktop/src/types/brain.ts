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
 * Practice mode the session is running in. Mirrors
 * `brain::session::PracticeMode`, which serialises in `snake_case`.
 *
 * - `warmup` — AI monitors silently, only surfaces readiness indicators.
 * - `practice` — default: full phrase-level coaching.
 * - `run_through` — AI stays silent, full recap at end only.
 */
export type PracticeMode = "warmup" | "practice" | "run_through";

/**
 * Display metadata for each `PracticeMode`. Rendering code imports this
 * so copy stays in one place and matches the product voice agreed in
 * story #21.
 */
export const PRACTICE_MODES: Array<{
  value: PracticeMode;
  label: string;
  description: string;
}> = [
  {
    value: "warmup",
    label: "Warm-up",
    description: "Silent monitoring. Readiness cues only — no critique.",
  },
  {
    value: "practice",
    label: "Practice",
    description: "Full coaching with phrase-level feedback and tips.",
  },
  {
    value: "run_through",
    label: "Run-through",
    description: "Performance mode. Silent now, full recap at the end.",
  },
];

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
/** Score position tracking (measure + beat). Mirrors `brain::follower::ScorePosition`. */
export interface ScorePosition {
  measure_number: number;
  beat: number;
  /** Section label (e.g. "Verse"), when the score provides one. */
  section_name?: string | null;
  /** MIDI note expected at this position, when known. */
  expected_note?: number | null;
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
  score_position?: ScorePosition;
  /** Tone-quality descriptor, when tone analysis ran over the phrase audio. */
  tone?: ToneDescriptor | null;
  /** Rolling key/mode estimate as of this phrase. Mirrors `theory::KeyEstimate`. */
  key?: KeyEstimate | null;
}

/**
 * Multi-dimensional tone descriptor (each axis 0..1). Mirrors
 * `tone::ToneDescriptor`. Deliberately not a single score — these are the
 * qualities a teacher names.
 */
export interface ToneDescriptor {
  brightness: number;
  warmth: number;
  air_noise: number;
  core_clarity: number;
  vibrato_quality: number;
}

/**
 * Diatonic mode. Mirrors `theory::Mode` (serialises in `snake_case`).
 */
export type Mode =
  | "ionian"
  | "dorian"
  | "phrygian"
  | "lydian"
  | "mixolydian"
  | "aeolian"
  | "locrian";

/**
 * Detected key/mode. Mirrors `theory::KeyEstimate`. `tonic` is a pitch class
 * 0–11 (C = 0); `confidence`/`margin` let the UI hedge on shaky calls.
 */
export interface KeyEstimate {
  tonic: number;
  mode: Mode;
  confidence: number;
  margin: number;
}

const PITCH_CLASS_NAMES = [
  "C",
  "C#",
  "D",
  "D#",
  "E",
  "F",
  "F#",
  "G",
  "G#",
  "A",
  "A#",
  "B",
];

const MODE_LABELS: Record<Mode, string> = {
  ionian: "major",
  aeolian: "minor",
  dorian: "Dorian",
  phrygian: "Phrygian",
  lydian: "Lydian",
  mixolydian: "Mixolydian",
  locrian: "Locrian",
};

/** Human label for a key, e.g. `"C major"`, `"G Mixolydian"`. */
export function keyName(key: KeyEstimate): string {
  const pc = PITCH_CLASS_NAMES[((key.tonic % 12) + 12) % 12];
  return `${pc} ${MODE_LABELS[key.mode]}`;
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
  /** Session-level tone aggregate, when tone analysis ran. */
  session_tone?: ToneDescriptor | null;
  /** Session-level key/mode, when detected confidently. `theory::KeyEstimate`. */
  session_key?: KeyEstimate | null;
}

/**
 * An instrument the user can choose at session start. Matches
 * `commands::InstrumentInfo` on the Rust side.
 *
 * Rust serialises with `rename_all = "camelCase"`, so `freqMinHz` /
 * `freqMaxHz` on the wire map cleanly to the JSON fields the frontend
 * reads. `emoji` is sourced from `profiles/*.json` — it's the string
 * stored there, not a lookup.
 */
export interface InstrumentInfo {
  name: string;
  family: string;
  freqMinHz: number;
  freqMaxHz: number;
  vibratoToleranceCents: number;
  emoji: string;
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

/** Session summary for history listing. Matches `commands::SessionSummaryDto`. */
export interface SessionSummaryDto {
  id: string;
  instrument: string;
  started_at: string; // RFC 3339
  duration_secs: number;
  phrase_count: number;
}

/** Full session with recap for detail view. Matches `commands::StoredSessionDto`. */
export interface StoredSessionDto {
  id: string;
  started_at: string; // RFC 3339
  ended_at: string; // RFC 3339
  recap: SessionRecap;
}

/** Practice statistics dashboard. Matches `commands::PracticeStatsDto`. */
export interface PracticeStatsDto {
  total_sessions: number;
  total_time_secs: number;
  sessions_this_week: number;
  avg_session_length_secs: number;
  trend: "up" | "down" | "stable";
}

/** Score library entry metadata. Matches `commands::ScoreLibraryEntryDto`. */
export interface ScoreLibraryEntry {
  id: string; // UUID
  title: string;
  composer: string | null;
  source_filename: string;
  added_at: string; // RFC 3339
  last_practiced_at: string | null; // RFC 3339
  part_index: number;
  duration_measures: number;
}

/** A loaded score ready for practice. Matches `commands::LoadedScoreDto`. */
export interface LoadedScore {
  entry: ScoreLibraryEntry;
  music_xml: string;
}
