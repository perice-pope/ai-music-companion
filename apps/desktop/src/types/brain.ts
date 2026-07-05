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

/** One guided-lesson drill as rendered by the UI. Mirrors `commands::DrillDto`. */
export interface DrillDto {
  index: number;
  drill_count: number;
  kind: string;
  label: string;
  tempo_bpm: number;
  difficulty: number;
  music_xml: string;
  target_len: number;
  /** Roots as pitch classes (0-11) in play order — RV's shuffled key cells. */
  root_pitch_classes: number[];
}

/** Trimmed drill grade. Mirrors `commands::DrillScoreDto`. */
export interface DrillScoreDto {
  accuracy: number;
  pitch_accuracy: number;
  timing_accuracy: number;
  correct: number;
  total: number;
}

/** Lesson recap. Mirrors `brain::coach::LessonRecap`. */
export interface LessonRecapDto {
  drill_labels: string[];
  drill_accuracies: number[];
  start_difficulty: number;
  end_difficulty: number;
}

/**
 * One step of the lesson state machine. Mirrors `commands::LessonStepDto`:
 * `score` grades what was just played (absent on start); then either the next
 * `drill` or the final `recap`.
 */
export interface LessonStepDto {
  seed: number;
  score: DrillScoreDto | null;
  drill: DrillDto | null;
  recap: LessonRecapDto | null;
}

/** The derived "your sound" identity. Mirrors `brain::mirror::SoundProfile`. */
export interface SoundProfileDto {
  sessions_counted: number;
  mode_lean: "minor" | "major" | "balanced" | null;
  feel: "swung" | "straight" | "mixed" | null;
  comparison: { label: string; source: string } | null;
  confidence: number;
  derived_at: number;
}

/** Mirrors `commands::SoundMirrorDto`. */
export interface SoundMirrorDto {
  profile: SoundProfileDto | null;
  sessions_seen: number;
}

/** Mastery state of one wheel cell. Mirrors `brain::wheel::KeyState`. */
export type KeyState = "none" | "learning" | "owned";

/** Trend of a measured dimension. Mirrors `brain::wheel::Trend`. */
export type WheelTrend = "improving" | "steady" | "slipping" | "unknown";

/** One wheel cell. Mirrors `brain::wheel::KeyCell`. */
export interface KeyCellDto {
  tonic: number;
  state: KeyState;
  attempts: number;
  best_accuracy: number;
  scales: string[];
}

/** The mastery wheel snapshot. Mirrors `brain::wheel::WheelView`. */
export interface WheelViewDto {
  cells: KeyCellDto[];
  intonation_trend: WheelTrend;
  tone_trend: WheelTrend;
  total_owned: number;
}

/** A concrete change a chip applies. Mirrors `brain::coach::VariationDelta`
 * (serde tag = "kind", snake_case). The frontend never constructs these beyond
 * echoing a chip's own delta back. */
export type VariationDelta =
  | { kind: "reshuffle_roots" }
  | { kind: "bump_difficulty"; by: number }
  | { kind: "different_scale" }
  | { kind: "try_pattern" }
  | { kind: "toggle_direction" };

/** One tappable chip. Mirrors `brain::coach::ChipSpec`. */
export interface ChipSpec {
  label: string;
  delta: VariationDelta;
}

/** One dot on the staff. Mirrors `brain::score::cellstaff::CellStaffNote`. */
export interface CellStaffNoteDto {
  midi: number;
  start_beat: number;
  duration_beats: number;
  step: number;
  accidental: number | null;
}

/** The dot-staff view. Mirrors `brain::score::cellstaff::CellStaffView`. */
export interface CellStaffViewDto {
  fifths: number;
  beats_per_measure: number;
  total_beats: number;
  notes: CellStaffNoteDto[];
}

/** One exploration rep. Mirrors `commands::ExploreDto`. */
export interface ExploreDto {
  label: string;
  music_xml: string;
  chips: ChipSpec[];
  root_pitch_classes: number[];
  staff: CellStaffViewDto;
  can_undo: boolean;
}

/** A semantic note-edit gesture. Mirrors `brain::coach::NoteEdit` — the
 * frontend never computes pitches, only gestures. */
export type NoteEdit =
  | { kind: "staff_steps"; by: number }
  | { kind: "semitones"; by: number }
  | { kind: "octaves"; by: number }
  | { kind: "remove" };

/**
 * Where a reveal's wording came from. Mirrors
 * `brain::connections::RevealSource` (`#[serde(rename_all = "snake_case")]`).
 * Slice 1 only ever emits `"grounded"`.
 */
export type RevealSource = "grounded" | "llm_grounded";

/**
 * A real-world music connection for what the player is doing right now.
 * Mirrors `brain::connections::Reveal`. Returned by the `get_reveal` command.
 */
export interface Reveal {
  /** The musical concept detected, e.g. "G Dorian". */
  concept: string;
  /** The grounded real-world connection, e.g. `Miles Davis — "So What"`. */
  connection: string;
  /** One short line on why. */
  why: string;
  /** Provenance of the wording. */
  source: RevealSource;
  /** Tonic pitch class (0-11) this reveal was generated for (#266). */
  tonic: number;
  /** Normalized (lowercased) mode this reveal was generated for, e.g. "dorian". */
  mode: string;
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
  /** Onset timestamps (seconds) retained for session-level groove analysis. */
  onsets_secs?: number[];
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

/**
 * Tuning tendency of a single scale degree relative to the tonic. Mirrors
 * `theory::DegreeTendency`. `mean_cents > 0` = sharp, `< 0` = flat.
 */
export interface DegreeTendency {
  semitones_from_tonic: number;
  mean_cents: number;
  count: number;
}

/**
 * Session-level intonation summary. Mirrors `theory::IntonationSummary`.
 * Only present on a recap when enough notes were observed to report honestly.
 */
export interface IntonationSummary {
  note_count: number;
  mean_cents: number;
  mean_abs_cents: number;
  in_tune_ratio: number;
  tendencies: DegreeTendency[];
}

/**
 * Session-level groove descriptor (tempo, swing, timing). Mirrors
 * `groove::GrooveDescriptor`. `tempo_bpm` / `swing_ratio` are `null` when there
 * weren't enough onsets to estimate them.
 */
export interface GrooveDescriptor {
  tempo_bpm?: number | null;
  swing_ratio?: number | null;
  mean_ioi_secs: number;
  timing_consistency: number;
  onset_count: number;
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
 * The session's unified musical fingerprint — what we measured about this
 * session's musicianship. Mirrors `brain::fingerprint::MusicalFingerprint`.
 *
 * Each dimension is present only when its evidence gate passed on the backend
 * (enough distinct pitch classes + confident fit for `key`; enough notes for
 * `intonation`; enough onsets for `groove`; at least one toned phrase for
 * `tone`). An absent dimension means "we didn't measure this honestly", never
 * "the value was zero". This is the single contract the personalization /
 * cultural-relevance layer consumes.
 */
export interface MusicalFingerprint {
  /** Session-level tone aggregate, when tone analysis ran. */
  tone?: ToneDescriptor | null;
  /** Session-level key/mode, when detected confidently. `theory::KeyEstimate`. */
  key?: KeyEstimate | null;
  /** Session-level intonation summary, when enough notes were observed. */
  intonation?: IntonationSummary | null;
  /** Session-level groove (tempo/swing/timing), when enough onsets were observed. */
  groove?: GrooveDescriptor | null;
}

/**
 * A confidence-gated idiom match for the session. Mirrors
 * `brain::idiom_recap::IdiomMatch` (re-exported from `idiom::IdiomMatch`).
 *
 * Computed **fully offline** on-device: the session audio is embedded and
 * matched against a bundled corpus, with matches below the engine's similarity
 * threshold dropped. `similarity` is a real cosine proximity in `[-1, 1]` — a
 * grounded "reminds me of" signal, never an asserted fact. The UI surfaces
 * these quietly, hedged, and only when present.
 */
export interface IdiomMatch {
  /** Human-facing idiom name, e.g. "Bebop line". */
  label: string;
  /** Broad family / genre, e.g. "jazz". */
  genre: string;
  /** A representative artist (named, not hosted). */
  exemplar_artist: string;
  /** Rough era, e.g. "1940s-50s". */
  era: string;
  /** Cosine similarity to the session audio, in [-1, 1]. */
  similarity: number;
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
  /**
   * The session's musical fingerprint (tone, key, intonation, groove), when
   * anything was measured. `null`/absent when every dimension's gate failed.
   */
  fingerprint?: MusicalFingerprint | null;
  /**
   * A short, hedged "flavour" line grounded in the reliable signal the app
   * measures — the key's mode + the groove's swing ratio (Rust
   * `brain::coaching::theory_flavour`). Drives the recap's "Flavour:" line.
   * `null`/absent when there's no clear signal ("silence > lies"). This
   * replaces the placeholder idiom corpus (#208) as the displayed flavour;
   * {@link idiom_notes} is kept for the real-corpus future but no longer
   * rendered as the flavour.
   */
  flavour?: string | null;
  /**
   * Confidence-gated, offline idiom flavours for the session — grounded,
   * hedged "reminds me of" notes. Empty/absent when nothing cleared the
   * engine's gate ("silence > lies"). `serde(default)` on the Rust side, so
   * older recaps load with this absent. Kept as plumbing for the real-corpus
   * future (#208); no longer rendered as the recap's "Flavour:" line.
   */
  idiom_notes?: IdiomMatch[];
  /**
   * Grounded, hedged cross-genre connections relating what the student played
   * (the measured {@link fingerprint}) to the genres/artists in their stated
   * taste profile — the Phase 4 cultural bridge. Each string is style/feel/
   * technique-level only, never a hard claim about a specific recording.
   *
   * Populated by the Rust core only when a taste profile existed AND the
   * session produced enough measured signal to ground a connection AND the
   * coach actually returned one. Empty/absent otherwise (cold start, thin
   * signal, or the coach chose silence). The UI shows it only when present.
   */
  connections?: string[];
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

/** Payload of the `accompaniment-status` Tauri event ("Play with me"). */
export interface AccompanimentStatus {
  playing: boolean;
}

/** A pickable key — display name plus what's needed to pin the band to it. Mirrors `brain::perception::KeyOption`. */
export interface KeyOption {
  /** Tonic pitch class, 0–11 (C = 0). */
  tonic: number;
  /** Minor (Aeolian) vs. major (Ionian). */
  minor: boolean;
  /** Full name, e.g. "E minor". */
  name: string;
}

/** The detected key, with its honest relative alternative. Mirrors `brain::perception::KeySnapshot`. */
export interface KeySnapshot {
  /** Tonic pitch class, 0–11 (C = 0). */
  tonic: number;
  /** Mode label, e.g. "major" / "minor" / "Mixolydian". */
  mode: string;
  /** Full name, e.g. "G major". */
  name: string;
  /** Best-fit confidence, 0–1. */
  confidence: number;
  /** Relative-key reading the player might mean, e.g. "E minor" for "G major". */
  alternative: KeyOption | null;
}

/** Payload of the `perception` Tauri event — what the app hears live. Mirrors `brain::perception::PerceptionSnapshot`. */
export interface PerceptionSnapshot {
  tempo_bpm: number | null;
  swing_ratio: number | null;
  /** A confident, steady pulse is present. */
  locked: boolean;
  key: KeySnapshot | null;
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

/**
 * Coarse, self-reported experience level. Mirrors
 * `brain::store::ExperienceLevel`, which serialises in `snake_case`. Shapes
 * coaching vocabulary/depth — never a grade.
 */
export type ExperienceLevel = "beginner" | "intermediate" | "advanced";

/** Display metadata for each `ExperienceLevel`. Calm, non-judgemental copy. */
export const EXPERIENCE_LEVELS: Array<{
  value: ExperienceLevel;
  label: string;
  description: string;
}> = [
  {
    value: "beginner",
    label: "Just starting",
    description: "New to this — or coming back after a break.",
  },
  {
    value: "intermediate",
    label: "Getting comfortable",
    description: "Fundamentals are solid; building repertoire.",
  },
  {
    value: "advanced",
    label: "Going deep",
    description: "Fluent — refining musicianship and advanced technique.",
  },
];

/**
 * The student's stated taste profile. Mirrors `brain::store::TasteProfile`.
 *
 * Stated preferences (who the student is), distinct from the measured
 * {@link MusicalFingerprint} (what they played). Captured at onboarding, stored
 * locally, and synced only if the user opts in. This is the data/capture
 * contract only — the relevance/coaching consumption lives elsewhere.
 */
export interface TasteProfile {
  genres: string[];
  artists: string[];
  goals: string[];
  experience: ExperienceLevel;
  is_under_13: boolean;
}

/** A fresh, empty taste profile — the cold-start shape. */
export function emptyTasteProfile(): TasteProfile {
  return {
    genres: [],
    artists: [],
    goals: [],
    experience: "beginner",
    is_under_13: false,
  };
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
