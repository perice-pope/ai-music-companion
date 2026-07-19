import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  CoachingTip,
  DrillDto,
  ExploreDto,
  DrillScoreDto,
  KeyOption,
  NoteEdit,
  VariationDelta,
  LessonRecapDto,
  LessonStepDto,
  PerceptionSnapshot,
  PhraseSummary,
  PracticeMode,
  Reveal,
  SessionRecap,
  ScoreLibraryEntry,
  ScorePosition,
  LoadedScore,
  StarterItem,
} from "../types/brain";
import type { ChartEntry } from "../types/brain";
import { useAudioStore } from "./audioStore";

/**
 * Screen routing enum — keeps Free Play as a state-machine in the
 * store instead of pulling in react-router for four screens (see
 * design doc §2).
 */
export type AppScreen =
  | "selector"
  | "score-picker"
  | "session"
  | "recap"
  | "history"
  | "connections";

/**
 * Finite lifecycle states for a session. `recap_ready` exists so we
 * can distinguish "Rust returned the recap and we're about to nav"
 * from "already on the recap screen" in PR 2+.
 */
export type SessionStatus =
  | "idle"
  | "starting"
  | "listening"
  | "ending"
  | "recap_ready";

/**
 * A coaching tip queued in the panel. UUID keys keep React's
 * reconciler stable when tips rotate through.
 */
export interface QueuedTip {
  id: string;
  tip: CoachingTip;
  receivedAt: number;
  phraseIndex: number;
}

/**
 * A real-world music "reveal" queued for the card. Mirrors {@link QueuedTip};
 * UUID keys keep React's reconciler stable as reveals rotate through.
 */
export interface QueuedReveal {
  id: string;
  reveal: Reveal;
  receivedAt: number;
  phraseIndex: number;
}

/** A live note verdict class from the backend (#337 S2). */
export type NoteVerdictKind = "hit" | "near" | "missed";

const EMPTY_VERDICTS = {
  hit: 0,
  near: 0,
  missed: 0,
  recent: [] as NoteVerdictKind[],
};

/** Most recent verdicts kept for the strip — enough to read a trend. */
const VERDICT_RECENT_CAP = 12;

/** One rolling jam-lane chip (#349 T4a) — view-local; the recap's
 * authoritative chart comes from the backend recorder. */
export interface LaneEntry {
  /** Display label; empty for the honest "several notes" state. */
  label: string;
  rootPc: number | null;
  /** Quality key for tap-to-row (snake_case), when labeled. */
  quality: string | null;
  confidence: number;
  unresolved: boolean;
}

/** Lane length: the last ~8 things the room played. */
const LANE_CAP = 8;

/** Fold a perception snapshot into the rolling lane: a new entry on chord
 * identity change or on entering the unresolved state; a ringing label
 * refreshes its confidence in place; SILENCE arms a release so a re-strike
 * of the same chord lands again — mirroring the backend recorder's change
 * discipline so lane and chart never disagree. */
function pushLane(
  lane: LaneEntry[],
  ringing: boolean,
  p: import("../types/brain").PerceptionSnapshot,
): { lane: LaneEntry[]; ringing: boolean } {
  const last = lane[lane.length - 1];
  const chord = p.chord ?? null;
  // A labeled chord without a quality key can't be rowed and the backend
  // recorder drops it (wire-compat state only, unreachable live) — treat
  // it the same here so lane and chart can never disagree.
  if (chord && chord.quality == null) {
    return { lane, ringing: false };
  }
  if (chord) {
    // Identity = (root, quality), mirroring the backend recorder's rule —
    // a slash arriving mid-ring ("C" → "C/E") refreshes the chip in place,
    // never a duplicate entry the recap chart won't have.
    const sameIdentity =
      ringing &&
      last &&
      !last.unresolved &&
      last.rootPc === chord.root_pc &&
      last.quality === (chord.quality ?? null);
    if (sameIdentity) {
      if (last.confidence === chord.confidence && last.label === chord.label) {
        return { lane, ringing: true };
      }
      return {
        lane: [
          ...lane.slice(0, -1),
          { ...last, confidence: chord.confidence, label: chord.label },
        ],
        ringing: true,
      };
    }
    return {
      lane: [
        ...lane,
        {
          label: chord.label,
          rootPc: chord.root_pc,
          quality: chord.quality ?? null,
          confidence: chord.confidence,
          unresolved: false,
        },
      ].slice(-LANE_CAP),
      ringing: true,
    };
  }
  if (p.hearing_polyphony) {
    if (ringing && last?.unresolved) {
      return { lane, ringing: true }; // one entry per stretch
    }
    return {
      lane: [
        ...lane,
        {
          label: "",
          rootPc: null,
          quality: null,
          confidence: 0,
          unresolved: true,
        },
      ].slice(-LANE_CAP),
      ringing: true,
    };
  }
  // Silence / single line: nothing to add, and the next identical chord is
  // a fresh strike.
  return { lane, ringing: false };
}

/**
 * One playable track of a multi-part MIDI file (field names mirror the Rust
 * `MidiPartDto`). `track_index` is the file's original track number — pass it
 * back to `importMidiFromFile` to import just that part.
 */
export interface MidiPart {
  track_index: number;
  name: string;
  note_count: number;
}

/**
 * Result of an audio import: the new library entry plus a calm quality
 * signal (field names mirror the Rust `ImportedAudioDto`). We surface
 * approximate-ness; we never show a fake accuracy score.
 */
export interface ImportedAudio {
  entry: ScoreLibraryEntry;
  note_count: number;
  mean_confidence: number;
  polyphony: number;
  /** Input looks polyphonic — basic-pitch is monophonic-first. */
  polyphonic: boolean;
  /** Transcription confidence looks weak. */
  low_confidence: boolean;
  /** Notes the model was unsure of — "caught N notes, M uncertain" (#331). */
  uncertain_count: number;
  /** Honesty-gate verdict, classified in the Rust core (#331). */
  verdict: "mono" | "borderline" | "full_mix";
}

/**
 * Result of recognizing a sheet-music PDF on-device (OMR). Field names mirror
 * the Rust `RecognizedPdfDto`. Recognition stores nothing: `music_xml` is fed
 * back through `importMusicXmlFromFile` with the chosen part, so PDF import
 * reuses the same part-picker + library path as MusicXML import.
 */
export interface RecognizedPdf {
  /** The recognized MusicXML — import this with the chosen part index. */
  music_xml: string;
  /** Parts found, in score order — drives the same "which part?" picker. */
  parts: string[];
  /** Always true: a scan is approximate; the UI shows a "check it" note. */
  from_scan: boolean;
  /** The scan yielded almost nothing — warn the read likely failed. */
  low_content: boolean;
}

/**
 * The key-confidence bar at which surfaces treat the detected key as
 * ASSERTED (the "I hear" header drops its hedge, the reveal card dims on
 * contradiction, the opener captures its tonic). One constant so the
 * surfaces can never silently disagree (#419 S2b review MF5).
 */
export const KEY_ASSERT_CONFIDENCE = 0.55;

/** #421 S2: handoff follows for this long, then freezes. */
export const HANDOFF_FOLLOW_MS = 8_000;
/** #421 S2: follow sends at most once per this window… */
const FOLLOW_THROTTLE_MS = 1_000;
/** …and only when the reading moved at least this much. */
const FOLLOW_MIN_DELTA_BPM = 2;

const POCKET_TEMPO_KEY = "ai-music-companion:pocket-tempo";

/** #421 S1: the click tempo survives restarts (spec §4). Default 90. */
function loadPocketTempo(): number {
  try {
    const raw = localStorage.getItem(POCKET_TEMPO_KEY);
    const parsed = raw === null ? NaN : Number(raw);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : 90;
  } catch {
    return 90;
  }
}

function savePocketTempo(bpm: number): void {
  try {
    localStorage.setItem(POCKET_TEMPO_KEY, String(bpm));
  } catch {
    // localStorage unavailable — the tempo still works for this session.
  }
}

/** All the state the free-play flow needs. */
export interface PracticeState {
  // Routing ---------------------------------------------------------------
  screen: AppScreen;

  // Session lifecycle -----------------------------------------------------
  status: SessionStatus;
  sessionId: string | null;
  instrumentName: string | null;
  /** Opaque id for the current instrument segment (updated on switch). */
  segmentId: string | null;
  startedAtMs: number | null;
  /** Ticked by `tick()` on a 1Hz setInterval. Not derived per-render. */
  elapsedSecs: number;

  // Live session data -----------------------------------------------------
  phrases: PhraseSummary[];
  tipQueue: QueuedTip[];
  /** Real-world music reveals queued for the reveal card (most recent shown). */
  revealQueue: QueuedReveal[];
  /**
   * Distinct reveals unlocked in the Learner Model collection (#253 S3), as
   * last reported by `record_reveal`. `null` until the first unlock this run.
   */
  collectionCount: number | null;

  // Guided lesson (#254) --------------------------------------------------
  /** The drill currently on screen, or null when no lesson is running. */
  lessonDrill: DrillDto | null;
  /** Grade of the drill just submitted (shown between drills / at the end). */
  lessonScore: DrillScoreDto | null;
  /** Final recap once the lesson completes. */
  lessonRecap: LessonRecapDto | null;
  /** A drill submit is in flight — guards the double-tap (#254 review M2). */
  lessonSubmitting: boolean;
  /**
   * A calm, human notice from the lesson backend (e.g. "I didn't catch that
   * yet…" on an eager grade-tap). Shown under the drill header; cleared on
   * the next successful step.
   */
  lessonNotice: string | null;

  // Free-play exploration (#255) ------------------------------------------
  /** The variation on the free-play surface, or null when just listening. */
  explore: ExploreDto | null;
  /** #419 S1 — the Openers panel's items being composed. */
  openerItems: StarterItem[];
  /** Live preview of the composed opener (pure — no session state). */
  openerPreview: ExploreDto | null;
  /** Calm refusal from the opener compiler, shown in the panel. */
  openerNotice: string | null;
  /** #419 S2b: the tonic pc the LAST preview captured (confident live
   * key at refresh time; null = C). Begin sends this, never a fresh
   * read — the preview IS the exercise. */
  openerTonic: number | null;
  /** #419 S2b: the recipe-level direction setting (a setting, not an
   * item — it survives item edits, resets after Begin/session end). */
  openerDirection: "forward" | "reversed" | "varied";
  /** The direction the painted preview was built with — Begin sends
   * THIS (with openerTonic): Begin plays what you SEE, not a setting a
   * mid-flight refresh hasn't painted yet (review MF2). */
  openerPreviewedDirection: "forward" | "reversed" | "varied";
  /** Monotonic refresh token — replaces the S1 items-identity guard,
   * which two direction-triggered refreshes could defeat (same array
   * identity). Only the latest refresh may commit ANY opener state. */
  _openerRefreshSeq: number;

  // Recap -----------------------------------------------------------------
  recap: SessionRecap | null;
  recapError: string | null;

  // Score mode (story-score-mode PR 1) ------------------------------------
  activeScore: ScoreLibraryEntry | null;
  /**
   * Raw MusicXML for `activeScore`, fetched lazily when a score is
   * selected. `null` until loaded — `ScoreView` renders nothing without
   * it. Kept here (not just in the component) so it survives navigation
   * between score-picker and session.
   */
  activeScoreXml: string | null;
  scoreLibrary: ScoreLibraryEntry[];
  cursorPosition: ScorePosition | null;
  /**
   * An exploration created from the RECAP screen (#337 S5 — "row this
   * measure through 12 keys") waiting for the next session to start. It
   * survives returnToSelector (which clears `explore`) and is promoted to
   * `explore` when the session reaches listening.
   */
  pendingExplore: ExploreDto | null;
  /** #337 S5: row one measure of the practiced score through 12 keys. */
  exploreMeasure: (measureNumber: number) => Promise<void>;
  /**
   * Calm inline notice for the recap's RV-bridge buttons (#337 S5): a
   * refusal ("measure 9 is all rests") must never replace the recap the
   * way recapError does — the summary stays, the notice explains.
   */
  bridgeNotice: string | null;
  /**
   * Live note-verdict tally for the current score session (#337 S2):
   * running counts plus the most recent verdicts (newest last, capped) for
   * the strip. Reset when a session starts (the only consumer mounts inside
   * a session, so stragglers after session end are invisible until then).
   */
  noteVerdicts: {
    hit: number;
    near: number;
    missed: number;
    recent: NoteVerdictKind[];
  };
  /** Fold one backend `note-verdict` event into the tally. */
  recordNoteVerdict: (verdict: NoteVerdictKind) => void;

  // Follow-me accompaniment ("Play with me") ------------------------------
  /**
   * Whether the backing band is currently playing. Authoritative source is
   * the backend's `accompaniment-status` event (the band may stop on its own,
   * e.g. at session end), so the toggle reflects this rather than optimistic
   * local state. Per-session, not a saved pref.
   */
  accompanimentPlaying: boolean;
  /**
   * Live "what the app hears" snapshot (tempo / feel / key + confidence) from
   * the backend `perception` event, updated ~8 Hz during a session. `null`
   * before anything is heard. Per-session, not persisted.
   */
  perception: PerceptionSnapshot | null;
  /**
   * #349 T4a "Listen to the room": jam-along mode. External music is the
   * SIGNAL — the chord lane replaces the free-play centerpiece. Deliberate
   * per session (reset at start); the mic pipeline is unchanged.
   */
  listenToRoom: boolean;
  /**
   * The rolling jam lane (#349 T4a): the last few chord labels the room
   * played, accumulated client-side from perception changes for display.
   * The RECAP's authoritative chart comes from the backend recorder.
   */
  chordLane: LaneEntry[];
  /** The last lane entry is still ringing (no silence since it landed). */
  laneRinging: boolean;
  /** Session time (secs) the room mode was FIRST enabled — the recap
   * sketch filters to entries after this, so pre-toggle practice chords
   * never masquerade as "what the room played". */
  jamStartSecs: number | null;
  /**
   * The finished jam's chord chart (#349 T4a), fetched from the backend at
   * session end when the room mode was on — the recap sketch's data.
   */
  jamChart: ChartEntry[] | null;
  /**
   * Whether the user has pinned the band's key (overriding auto-detection).
   * Optimistic UI mirror of the backend override — set on the override actions,
   * cleared on auto / session end. Per-session, not persisted.
   */
  keyPinned: boolean;
  /**
   * The key the band is pinned to while `keyPinned` (so the panel shows what the
   * band actually plays, not the still-changing auto-detected key). `null` when
   * not pinned. Always a concrete major/minor key.
   */
  pinnedKey: KeyOption | null;

  // UI prefs (persisted) --------------------------------------------------
  coachingEnabled: boolean;
  /**
   * Current practice mode. Persisted so a user's last choice survives
   * a restart. Defaults to `"practice"` on first run (matches the Rust
   * `PracticeMode::default()`).
   */
  practiceMode: PracticeMode;

  // Actions ---------------------------------------------------------------
  loadScoreFromFile: (path: string) => Promise<void>;
  /**
   * Import a MIDI file by its raw bytes (read on the frontend) and make it
   * the active score. The backend parses the MIDI, converts it to canonical
   * MusicXML, and stores it — see `import_midi_file`.
   */
  importMidiFromFile: (
    sourceFilename: string,
    bytes: number[],
    trackIndex?: number,
  ) => Promise<ScoreLibraryEntry>;
  /**
   * List the playable tracks of a MIDI file (read-only, nothing stored) so
   * the UI can ask which part to practice — the MIDI twin of
   * `listScoreParts`. Conductor and percussion tracks are omitted; a
   * single-part file returns one entry so the caller can skip the picker.
   */
  listMidiParts: (bytes: number[]) => Promise<MidiPart[]>;
  importAudioFromFile: (
    sourceFilename: string,
    bytes: number[],
  ) => Promise<ImportedAudio>;
  /**
   * List the instrument parts in a MusicXML file (read-only, nothing stored)
   * so the UI can ask which part the user wants to read and practice. A
   * single-part score returns one entry — the caller can skip the picker.
   */
  listScoreParts: (bytes: number[]) => Promise<string[]>;
  /**
   * Import a MusicXML file by its raw bytes and make it the active score. The
   * backend parses the file and reads `partIndex` (the user's choice from
   * `listScoreParts`; `0` for a single-part score) — see `import_musicxml_file`.
   */
  importMusicXmlFromFile: (
    sourceFilename: string,
    bytes: number[],
    partIndex: number,
  ) => Promise<ScoreLibraryEntry>;
  /**
   * Recognize a sheet-music PDF on-device (experimental beta — OMR). Returns the
   * recognized MusicXML and its parts; nothing is stored. The caller runs the
   * shared part picker and then imports the chosen part via
   * `importMusicXmlFromFile` — see `recognize_pdf_score`. Rejects with a calm
   * message when the beta is off or the engine isn't bundled.
   */
  recognizePdfFromFile: (
    sourceFilename: string,
    bytes: number[],
  ) => Promise<RecognizedPdf>;
  loadScoreFromId: (id: string) => Promise<void>;
  refreshScoreLibrary: () => Promise<void>;
  deleteScore: (id: string) => Promise<void>;
  clearActiveScore: () => void;
  startSession: (
    instrument: string,
    vibratoToleranceCents?: number,
    scoreId?: string,
  ) => Promise<void>;
  endSession: () => Promise<void>;
  switchInstrument: (
    name: string,
    vibratoToleranceCents?: number,
  ) => Promise<void>;
  pushPhrase: (phrase: PhraseSummary) => void;
  pushTip: (tip: CoachingTip, phraseIndex: number) => void;
  /**
   * Start the follow-me accompaniment ("Play with me"). Fires
   * `start_accompaniment`; the band stays silent until it locks onto the
   * player's pulse. Authoritative playing state arrives via the
   * `accompaniment-status` event, so this doesn't set `accompanimentPlaying`
   * optimistically — it surfaces an error if the command fails.
   */
  /** #214 S1b: the current library match, sticky per rule 0 (replaced
   * in place by a newer match, dimming handled by the chip, dismissal
   * quiets that score for the session). */
  pieceMatch: { scoreId: string; title: string } | null;
  /** Score ids the player dismissed this session — stay quiet. */
  dismissedPieceIds: string[];
  requestPieceMatch: () => Promise<void>;
  dismissPieceMatch: () => void;
  /** #214 S2: one tap from "you're playing X" to the score itself —
   * stages the matched score and lands on the selector. Resolves false
   * (staying put, quieting that id) if the score can't load. */
  openMatchedScore: () => Promise<boolean>;

  /** #421 S2: the click's personality — anchor holds, follow locks to
   * YOUR pulse, handoff follows then freezes ("now hold it"). */
  pocketMode: "anchor" | "follow" | "handoff";
  setPocketMode: (mode: "anchor" | "follow" | "handoff") => void;
  /** #421 S2: handoff's frozen tempo once the follow window closes. */
  pocketFrozenBpm: number | null;
  /** Internal follow-policy state (throttle + delta gate). */
  _pocketLastSentBpm: number | null;
  _pocketLastSentAt: number;
  _pocketFollowStartedAt: number;

  /** #421 S1: The Pocket — strict Anchor click state. */
  pocketPlaying: boolean;
  pocketTempo: number;
  pocketCountIn: boolean;
  setPocketTempo: (bpm: number) => void;
  setPocketCountIn: (on: boolean) => void;
  startPocket: () => Promise<void>;
  stopPocket: () => Promise<void>;
  setPocketStatus: (playing: boolean, tempoBpm: number) => void;

  startAccompaniment: () => Promise<void>;
  /** Stop the follow-me accompaniment. Fires `stop_accompaniment`. */
  stopAccompaniment: () => Promise<void>;
  /** Reflect the backend's `accompaniment-status` event. */
  setAccompanimentPlaying: (playing: boolean) => void;
  /** Reflect the backend's live `perception` event (what the app hears). */
  setPerception: (perception: PerceptionSnapshot | null) => void;
  setListenToRoom: (on: boolean) => void;
  /** #349 T4a — tap a lane chord: row it through 12 keys as block cells. */
  exploreChord: (rootPc: number, quality: string) => Promise<void>;
  /** #349 T3c — lift the chart's chord sequence and row it through 12
   * keys as stacked cells. Calm refusal under two distinct chords. */
  exploreProgression: () => Promise<void>;
  /** #341 — tap a measure MID-PRACTICE: row it through 12 keys without
   * ending the session (the in-practice half of the RV bridge; the recap
   * half is `exploreMeasure`, which parks a pending handoff instead). */
  exploreMeasureLive: (measureNumber: number) => Promise<void>;
  addOpenerItem: (item: StarterItem) => Promise<void>;
  removeOpenerItem: (index: number) => Promise<void>;
  setOpenerDirection: (direction: "forward" | "reversed" | "varied") => void;
  clearOpener: () => void;
  /** Internal: recompute the pure preview after any item change. */
  _refreshOpenerPreview: (items: StarterItem[]) => Promise<void>;
  beginOpener: () => Promise<void>;
  /** #419 S4: repopulate the builder from a saved recipe — items and
   * direction land together, then the EXISTING preview path re-voices
   * (no generation logic lives frontend-side). */
  applyOpenerRecipe: (
    items: StarterItem[],
    direction: "forward" | "reversed" | "varied",
  ) => Promise<void>;
  /** #419 S4: replay yesterday's opener EXACTLY — the backend reads the
   * stored seed/cell/tonic/direction from the exercise log. */
  beginOpenerRecall: () => Promise<void>;
  /** Pin the band to a specific key (correcting the auto-read). */
  setAccompanimentKey: (tonic: number, minor: boolean) => Promise<void>;
  /** Freeze the band on its current auto-detected key. */
  lockAccompanimentKey: () => Promise<void>;
  /** Resume automatic key-following. */
  clearAccompanimentKey: () => Promise<void>;
  /**
   * Live coaching loop: ask the backend for a tip on a just-completed phrase,
   * surface it in the tip panel, and persist it in the session recorder.
   *
   * Gated on `coachingEnabled` (the user's opt-in): when off we fire **no**
   * IPC at all — there's nothing to ask for, and the Rust-core airplane switch
   * (`NetworkPolicy::Offline`) would refuse anyway. When on, the backend may
   * still return `null` (rate-limited, API failure, or offline) — that means
   * "no tip", and we honor the silence rather than inventing one.
   */
  requestCoachingTip: (phrase: PhraseSummary) => Promise<void>;
  dismissTip: (id: string) => void;
  /** Push a reveal onto the queue (most-recent is shown). */
  pushReveal: (reveal: Reveal, phraseIndex: number) => void;
  /**
   * Ambient reveal loop: on a just-completed phrase, ask the backend whether a
   * real-world music "reveal" is due for the current key/mode. Curated +
   * offline (slice 1), so unlike coaching it needs no opt-in — but it only runs
   * during a live session and only when perception has a key. A `null` reply is
   * the honest "nothing to reveal right now".
   */
  requestReveal: (phrase: PhraseSummary) => Promise<void>;
  dismissReveal: (id: string) => void;
  /**
   * Start a guided lesson (#254): the backend builds an adaptive RV routine
   * from the Learner Model and returns drill 0. Requires a live session (the
   * mic must be running for the drills to be graded).
   */
  startLesson: () => Promise<void>;
  /** Grade the just-played drill and step to the next one (or the recap). */
  submitDrill: () => Promise<void>;
  /** Abandon the lesson (nothing persisted). */
  endLesson: () => Promise<void>;
  /** Clear the recap view after the lesson. */
  clearLessonRecap: () => void;
  /** Start exploring a sound (#255): seeds an RV variation from a key/mode. */
  startExplore: (tonic: number, mode: string) => Promise<void>;
  /** #285: lift the player's last phrase as a cell and row it through the
   * keys. Sets a calm notice when nothing liftable was played yet. */
  exploreLastPhrase: () => Promise<void>;
  /** Calm one-liner from the explore backend (e.g. "play a phrase first"). */
  exploreNotice: string | null;
  /** Apply a tapped chip's delta to the in-flight exploration. */
  applyChip: (delta: VariationDelta) => Promise<void>;
  /** Apply a semantic note edit (#292): the backend bakes the cell. */
  editExploreNote: (index: number, edit: NoteEdit) => Promise<void>;
  /** Undo the most recent explore edit. */
  undoExploreEdit: () => Promise<void>;
  /** Stop exploring. */
  endExplore: () => Promise<void>;
  setCursorPosition: (pos: ScorePosition | null) => void;
  tick: () => void;
  returnToSelector: () => void;
  setCoachingEnabled: (on: boolean) => void;
  /**
   * Update the mode. If a session is already running, this only updates
   * local state — the Rust side is notified on the next `switchInstrument`
   * (which now carries the mode). Callers that want an immediate mid-session
   * change should call `switchInstrument` with the current instrument after
   * setting the new mode.
   */
  setPracticeMode: (mode: PracticeMode) => void;
  goToHistory: () => void;
  /** Open the Connections & Privacy panel (networked-feature disclosure). */
  goToConnections: () => void;
}

/** Pitch-class names (sharps), index 0 = C. */
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

/** Build a displayable major/minor key option from a tonic + minor flag. */
function keyOptionFor(tonic: number, minor: boolean): KeyOption {
  const pc = PITCH_CLASS_NAMES[((tonic % 12) + 12) % 12];
  return { tonic, minor, name: `${pc} ${minor ? "minor" : "major"}` };
}

/** localStorage key for the coaching-on/off preference. */
const COACHING_PREF_KEY = "ai-music-companion:coaching-enabled";
/** localStorage key for the last-used practice mode. */
const PRACTICE_MODE_PREF_KEY = "ai-music-companion:practice-mode";

function loadCoachingPref(): boolean {
  try {
    const raw = localStorage.getItem(COACHING_PREF_KEY);
    // Default: DISABLED (off by default). AI coaching narration is a
    // networked feature, and the offline-first principle says every networked
    // feature is opt-in and starts off (see
    // `docs/architecture/offline-first-and-network-transparency.md`). On first
    // run the coach is served entirely by the on-device fallback. Only an
    // explicit "true" turns narration on; the choice is then persisted.
    return raw === "true";
  } catch {
    return false;
  }
}

function saveCoachingPref(on: boolean): void {
  try {
    localStorage.setItem(COACHING_PREF_KEY, on ? "true" : "false");
  } catch {
    // localStorage unavailable — silently ignore.
  }
}

function isPracticeMode(v: string | null): v is PracticeMode {
  return v === "warmup" || v === "practice" || v === "run_through";
}

function loadPracticeModePref(): PracticeMode {
  try {
    const raw = localStorage.getItem(PRACTICE_MODE_PREF_KEY);
    return isPracticeMode(raw) ? raw : "practice";
  } catch {
    return "practice";
  }
}

function savePracticeModePref(mode: PracticeMode): void {
  try {
    localStorage.setItem(PRACTICE_MODE_PREF_KEY, mode);
  } catch {
    // localStorage unavailable — silently ignore.
  }
}

/**
 * Tiny UUID generator that works in jsdom (crypto.randomUUID is available
 * in Vitest's jsdom >= 25). Falls back to a timestamp-based string if
 * for some reason crypto isn't present.
 */
function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `tip-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }
}

/**
 * Put an imported entry at the front of the library without duplicating it.
 * The backend dedups re-imports by content (#385) and hands back the
 * existing entry, so a blind prepend would show the same score twice until
 * the next refresh.
 */
function upsertScoreEntry(
  library: ScoreLibraryEntry[],
  entry: ScoreLibraryEntry,
): ScoreLibraryEntry[] {
  return [entry, ...library.filter((s) => s.id !== entry.id)];
}

export const usePracticeStore = create<PracticeState>((set, get) => ({
  screen: "selector",
  status: "idle",
  sessionId: null,
  instrumentName: null,
  segmentId: null,
  startedAtMs: null,
  elapsedSecs: 0,
  phrases: [],
  tipQueue: [],
  revealQueue: [],
  collectionCount: null,
  lessonDrill: null,
  lessonScore: null,
  lessonRecap: null,
  lessonSubmitting: false,
  lessonNotice: null,
  explore: null,
  openerItems: [],
  openerPreview: null,
  openerNotice: null,
  openerTonic: null,
  openerDirection: "forward" as const,
  openerPreviewedDirection: "forward" as const,
  _openerRefreshSeq: 0,
  exploreNotice: null,
  recap: null,
  recapError: null,
  activeScore: null,
  activeScoreXml: null,
  scoreLibrary: [],
  cursorPosition: null,
  pendingExplore: null,
  bridgeNotice: null,
  noteVerdicts: { ...EMPTY_VERDICTS, recent: [] },
  recordNoteVerdict: (verdict: NoteVerdictKind) =>
    set((state) => ({
      noteVerdicts: {
        ...state.noteVerdicts,
        [verdict]: state.noteVerdicts[verdict] + 1,
        recent: [...state.noteVerdicts.recent, verdict].slice(
          -VERDICT_RECENT_CAP,
        ),
      },
    })),
  accompanimentPlaying: false,
  perception: null,
  listenToRoom: false,
  chordLane: [],
  laneRinging: false,
  jamStartSecs: null,
  jamChart: null,
  keyPinned: false,
  pinnedKey: null,
  coachingEnabled: loadCoachingPref(),
  practiceMode: loadPracticeModePref(),

  loadScoreFromFile: async (path: string) => {
    try {
      const entry = await invoke<ScoreLibraryEntry>("import_score", { path });
      // Fetch the just-imported MusicXML so ScoreView can render it
      // without a second user action.
      const loaded = await invoke<LoadedScore>("get_score", { id: entry.id });
      set((state) => ({
        activeScore: entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
        scoreLibrary: upsertScoreEntry(state.scoreLibrary, entry),
      }));
    } catch (err) {
      throw new Error(`Failed to load score: ${err}`);
    }
  },

  importMidiFromFile: async (
    sourceFilename: string,
    bytes: number[],
    trackIndex?: number,
  ) => {
    try {
      const entry = await invoke<ScoreLibraryEntry>("import_midi_file", {
        sourceFilename,
        bytes,
        trackIndex: trackIndex ?? null,
      });
      // Load the freshly-imported MusicXML so ScoreView can render it
      // without a second user action (mirrors loadScoreFromFile).
      const loaded = await invoke<LoadedScore>("get_score", { id: entry.id });
      set((state) => ({
        activeScore: entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
        scoreLibrary: upsertScoreEntry(state.scoreLibrary, entry),
      }));
      return entry;
    } catch (err) {
      throw new Error(`Failed to import MIDI: ${err}`);
    }
  },

  importAudioFromFile: async (sourceFilename: string, bytes: number[]) => {
    try {
      const result = await invoke<ImportedAudio>("import_audio_file", {
        sourceFilename,
        bytes,
      });
      // Load the freshly-transcribed MusicXML so ScoreView can render it
      // without a second user action (mirrors importMidiFromFile).
      const loaded = await invoke<LoadedScore>("get_score", {
        id: result.entry.id,
      });
      set((state) => ({
        activeScore: result.entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
        scoreLibrary: upsertScoreEntry(state.scoreLibrary, result.entry),
      }));
      return result;
    } catch (err) {
      throw new Error(`Failed to import audio: ${err}`);
    }
  },

  listScoreParts: async (bytes: number[]) => {
    try {
      return await invoke<string[]>("list_score_parts", { bytes });
    } catch (err) {
      throw new Error(`Couldn't read the parts in that file: ${err}`);
    }
  },

  listMidiParts: async (bytes: number[]) => {
    try {
      return await invoke<MidiPart[]>("list_midi_parts", { bytes });
    } catch (err) {
      throw new Error(`Couldn't read the tracks in that file: ${err}`);
    }
  },

  importMusicXmlFromFile: async (
    sourceFilename: string,
    bytes: number[],
    partIndex: number,
  ) => {
    try {
      const entry = await invoke<ScoreLibraryEntry>("import_musicxml_file", {
        sourceFilename,
        bytes,
        partIndex,
      });
      // Load the freshly-imported MusicXML so ScoreView can render it without
      // a second user action (mirrors importMidiFromFile).
      const loaded = await invoke<LoadedScore>("get_score", { id: entry.id });
      set((state) => ({
        activeScore: entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
        scoreLibrary: upsertScoreEntry(state.scoreLibrary, entry),
      }));
      return entry;
    } catch (err) {
      throw new Error(`Failed to import MusicXML: ${err}`);
    }
  },

  recognizePdfFromFile: async (sourceFilename: string, bytes: number[]) => {
    try {
      // OMR is just a front-end that produces MusicXML — it stores nothing.
      // The caller hands the result to importMusicXmlFromFile with the chosen
      // part, reusing the same picker + library path as MusicXML import.
      return await invoke<RecognizedPdf>("recognize_pdf_score", {
        sourceFilename,
        bytes,
      });
    } catch (err) {
      // Preserve the backend's calm, specific message (beta off, engine not
      // bundled, unreadable scan) — it's written for the user.
      throw new Error(`${err}`);
    }
  },

  loadScoreFromId: async (id: string) => {
    try {
      // `get_score` returns the entry *and* its MusicXML — the library
      // list only carries metadata, so this is where the actual notes
      // come across the IPC boundary.
      const loaded = await invoke<LoadedScore>("get_score", { id });
      set({
        activeScore: loaded.entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
      });
    } catch (err) {
      throw new Error(`Failed to load score: ${err}`);
    }
  },

  refreshScoreLibrary: async () => {
    try {
      const library = await invoke<ScoreLibraryEntry[]>("list_scores");
      set({ scoreLibrary: library });
    } catch (err) {
      throw new Error(`Failed to refresh score library: ${err}`);
    }
  },

  deleteScore: async (id: string) => {
    try {
      await invoke("delete_score", { id });
      set((state) => ({
        scoreLibrary: state.scoreLibrary.filter((s) => s.id !== id),
        activeScore: state.activeScore?.id === id ? null : state.activeScore,
      }));
    } catch (err) {
      throw new Error(`Failed to delete score: ${err}`);
    }
  },

  clearActiveScore: () =>
    set({ activeScore: null, activeScoreXml: null, cursorPosition: null }),

  startSession: async (
    instrument: string,
    vibratoToleranceCents = 15.0,
    scoreId?: string,
  ) => {
    const { status } = get();
    if (status !== "idle") {
      throw new Error(
        `cannot start session from status=${status} — call endSession first`,
      );
    }
    set({
      status: "starting",
      recap: null,
      recapError: null,
      // Fresh verdict tally per session (#337 S2).
      noteVerdicts: { hit: 0, near: 0, missed: 0, recent: [] },
      // Jam mode is a deliberate per-session choice (#349 T4a).
      listenToRoom: false,
      chordLane: [],
      laneRinging: false,
      jamStartSecs: null,
      jamChart: null,
    });
    try {
      const sessionId = await invoke<string>("start_practice_session", {
        instrument,
        practiceMode: get().practiceMode,
        coachingEnabled: get().coachingEnabled,
        scoreId: scoreId || null,
      });
      set({
        status: "listening",
        // Promote a recap-made exploration (#337 S5) into the live session.
        explore: get().pendingExplore ?? null,
        pendingExplore: null,
        screen: "session",
        sessionId,
        instrumentName: instrument,
        segmentId: null,
        startedAtMs: Date.now(),
        elapsedSecs: 0,
        phrases: [],
        tipQueue: [],
        revealQueue: [],
        cursorPosition: null,
      });
      useAudioStore.getState().setInstrument(instrument, vibratoToleranceCents);
      // NOTE: `isListening` is *not* flipped here. It's driven by the
      // `audio-event` listener — the first event that arrives is
      // evidence that the backend pipeline actually opened the mic.
      // If the mic failed to open, the Rust side logs and continues
      // the session without emitting events, and `isListening` stays
      // false (so `PitchDisplay` shows the idle copy instead of lying
      // about a dead mic).
    } catch (err) {
      // Roll back to idle so the UI can retry. Store the error text so
      // the caller (or a future error banner) can surface it. Clear
      // any stale listening flag from a previous session.
      set({ status: "idle", recapError: String(err) });
      useAudioStore.getState().setListening(false);
      throw err;
    }
  },

  endSession: async () => {
    const { status } = get();
    if (status !== "listening") {
      throw new Error(
        `cannot end session from status=${status} — nothing to end`,
      );
    }
    set({ status: "ending" });
    // Mic pipeline is torn down on the Rust side by the time the
    // `end_practice_session` command even starts executing — flip the
    // listening flag immediately so `PitchDisplay` drops back to its
    // idle copy while the recap round-trip is in flight.
    useAudioStore.getState().setListening(false);
    const wasJam = get().listenToRoom;
    try {
      const recap = await invoke<SessionRecap>("end_practice_session");
      // #349 T4a: the jam's chord chart survives until the NEXT session
      // starts — fetch it now so the recap can sketch it. Best-effort:
      // a chart failure must not dent the recap.
      let jamChart: ChartEntry[] | null = null;
      if (wasJam) {
        try {
          const chart = await invoke<ChartEntry[]>("session_chord_chart");
          // Only what played AFTER the room mode came on — the player's
          // own pre-toggle practice is not "what the room played".
          const since = get().jamStartSecs ?? 0;
          const roomChart = chart.filter((e) => e.at_secs >= since);
          jamChart = roomChart.length > 0 ? roomChart : null;
        } catch {
          jamChart = null;
        }
      }
      set({
        status: "idle",
        screen: "recap",
        recap,
        recapError: null,
        sessionId: null,
        jamChart,
      });
    } catch (err) {
      // Recap failure: still exit the session, still go to the recap
      // screen — the screen has a fallback copy for `recapError`. This
      // matches the design doc: "never fail loudly on a recap."
      set({
        status: "idle",
        screen: "recap",
        recap: null,
        recapError: String(err),
        sessionId: null,
      });
    }
    // The backend tears the band down on session end and emits
    // `accompaniment-status { playing: false }`, but flip it here too so the
    // toggle resets immediately even if that event is missed. Perception also
    // stops once the session ends.
    set({
      accompanimentPlaying: false,
      perception: null,
      keyPinned: false,
      pinnedKey: null,
      // The backend finalizes a surviving lesson at session end; mirror that
      // here so a stale drill can't re-mount in the next session (#254 M1).
      lessonDrill: null,
      lessonScore: null,
      lessonRecap: null,
      lessonSubmitting: false,
      lessonNotice: null,
      explore: null,
      exploreNotice: null,
      // A half-built opener must not reappear in the NEXT session's panel.
      openerItems: [],
      openerPreview: null,
      openerNotice: null,
      openerTonic: null,
      openerDirection: "forward",
      openerPreviewedDirection: "forward",
      // #214 S1b: matches and dismissals are session-scoped.
      pieceMatch: null,
      dismissedPieceIds: [],
      // #421 S2: the click personality resets with the session.
      pocketMode: "anchor",
      pocketFrozenBpm: null,
      _pocketLastSentBpm: null,
    });
    // Round-3 review MF1: session end also invalidates in-flight opener
    // refreshes (separate set — this one derives from current state).
    set((s) => ({ _openerRefreshSeq: s._openerRefreshSeq + 1 }));
  },

  pieceMatch: null,
  dismissedPieceIds: [],

  requestPieceMatch: async () => {
    try {
      const dto = await invoke<{
        score_id: string;
        title: string;
        coherent_hits: number;
      } | null>("check_piece_match");
      if (!dto) {
        return; // None is the common answer — the chip holds (rule 0).
      }
      if (get().dismissedPieceIds.includes(dto.score_id)) {
        return; // The player said "not this one" — stay quiet.
      }
      if (get().activeScore?.id === dto.score_id) {
        // Score-mode echo (review MF3): "sounds like the piece you have
        // OPEN" is permanent redundant noise, not identification.
        return;
      }
      // Replace in place; never clear on a miss.
      set({ pieceMatch: { scoreId: dto.score_id, title: dto.title } });
    } catch {
      // Identification must never surface an error — silence is normal.
    }
  },

  dismissPieceMatch: () =>
    set((s) => ({
      pieceMatch: null,
      dismissedPieceIds: s.pieceMatch
        ? [...s.dismissedPieceIds, s.pieceMatch.scoreId]
        : s.dismissedPieceIds,
    })),

  openMatchedScore: async () => {
    const match = get().pieceMatch;
    if (!match) {
      return false;
    }
    let loaded: LoadedScore;
    try {
      // Load BEFORE leaving the session — a failure must not yank the
      // player to the selector for nothing.
      loaded = await invoke<LoadedScore>("get_score", {
        id: match.scoreId,
      });
    } catch {
      // Deleted mid-session, unreadable, whatever — stay put and stop
      // offering a door that doesn't open.
      set((s) => ({
        pieceMatch: null,
        dismissedPieceIds: [...s.dismissedPieceIds, match.scoreId],
      }));
      return false;
    }
    // S2 review MF1: this chip lives on LIVE session screens, and
    // returnToSelector alone would strand the backend session (hot mic
    // on the selector, AlreadyActive soft-lock on the next start). End
    // the session for real first — the recap persists backend-side
    // either way; the player chose the score over the recap screen.
    if (get().status === "listening") {
      set({ status: "ending" });
      useAudioStore.getState().setListening(false);
      try {
        await invoke("end_practice_session");
      } catch {
        // endSession's own precedent: "never fail loudly on a recap" —
        // the pipeline teardown precedes the command; still exit.
      }
    }
    get().returnToSelector();
    // Set AFTER returnToSelector (which clears activeScore) so the
    // staged score survives — the pendingExplore handoff pattern. The
    // extra resets mirror endSession's tail (session-scoped state that
    // returnToSelector doesn't own): openers, dismissals, the pocket.
    set({
      activeScore: loaded.entry,
      activeScoreXml: loaded.music_xml,
      cursorPosition: null,
      pieceMatch: null,
      dismissedPieceIds: [],
      openerItems: [],
      openerPreview: null,
      openerNotice: null,
      openerTonic: null,
      openerDirection: "forward",
      openerPreviewedDirection: "forward",
      pocketMode: "anchor",
      pocketFrozenBpm: null,
      _pocketLastSentBpm: null,
    });
    set((s) => ({ _openerRefreshSeq: s._openerRefreshSeq + 1 }));
    return true;
  },

  pocketMode: "anchor",
  pocketFrozenBpm: null,
  _pocketLastSentBpm: null,
  _pocketLastSentAt: 0,
  _pocketFollowStartedAt: 0,

  setPocketMode: (mode) =>
    set({
      pocketMode: mode,
      // A fresh personality starts a fresh follow window; anchor sends
      // nothing (AC5: switching mid-play stops the stream).
      pocketFrozenBpm: null,
      _pocketLastSentBpm: null,
      _pocketFollowStartedAt: Date.now(),
    }),

  pocketPlaying: false,
  pocketTempo: loadPocketTempo(),
  pocketCountIn: true,

  setPocketTempo: (bpm) => {
    savePocketTempo(bpm);
    set({ pocketTempo: bpm });
  },
  setPocketCountIn: (on) => set({ pocketCountIn: on }),

  startPocket: async () => {
    // Semantic settings only — clamping and validation are the backend's.
    await invoke("start_pocket", {
      tempoBpm: get().pocketTempo,
      beatsPerBar: 4,
      countIn: get().pocketCountIn,
    });
  },

  stopPocket: async () => {
    await invoke("stop_pocket");
  },

  // Authoritative playing state comes from the pocket-status event, same
  // discipline as the band (#421 rule: never optimistically flip).
  setPocketStatus: (playing, tempoBpm) =>
    set((s) => ({
      pocketPlaying: playing,
      // Review MF4: a FRESH click starts a fresh follow life — stale
      // frozen/sent state from a previous click made the drift line lie
      // and the delta gate block against a tempo the click no longer
      // holds. The follow window anchors at the click's real start.
      ...(playing && !s.pocketPlaying
        ? {
            pocketFrozenBpm: null,
            _pocketLastSentBpm: null,
            _pocketFollowStartedAt: Date.now(),
          }
        : {}),
      // The backend reports the CLAMPED tempo it actually plays — mirror
      // it so the pulse and label can never lie.
      pocketTempo: playing && tempoBpm > 0 ? tempoBpm : s.pocketTempo,
    })),

  startAccompaniment: async () => {
    // Authoritative `playing` comes from the `accompaniment-status` event, so we
    // don't optimistically flip it here — just fire the command and let the
    // event drive the chip. Errors are surfaced to the caller (the toggle shows
    // them); the live session must never break because the band failed to start.
    await invoke("start_accompaniment");
  },

  stopAccompaniment: async () => {
    await invoke("stop_accompaniment");
  },

  setAccompanimentPlaying: (playing) => set({ accompanimentPlaying: playing }),

  setPerception: (perception) => {
    // #421 S2: the follow policy — orchestration here, measurement in
    // perception, the seam in set_pocket_tempo. Confident, changed
    // (>=2 BPM), throttled (>=1 s) readings only; handoff freezes after
    // its window and the drift line reads frozen-vs-live.
    const s0 = get();
    const tempo = perception?.tempo_bpm ?? null;
    if (
      s0.pocketPlaying &&
      s0.pocketMode !== "anchor" &&
      tempo !== null &&
      // Review MF2: LOCKED means confident — an unconfident estimate
      // ships a non-null tempo the click must never chase (the band's
      // own filter, accompaniment.rs).
      perception?.locked === true &&
      tempo >= 40 &&
      tempo <= 220
    ) {
      const now = Date.now();
      if (
        s0.pocketMode === "handoff" &&
        s0.pocketFrozenBpm === null &&
        now - s0._pocketFollowStartedAt >= HANDOFF_FOLLOW_MS
      ) {
        // The handoff moment: "I've got your N. Now hold it."
        set({ pocketFrozenBpm: s0._pocketLastSentBpm ?? s0.pocketTempo });
      } else if (
        (s0.pocketMode === "follow" ||
          (s0.pocketMode === "handoff" && s0.pocketFrozenBpm === null)) &&
        now - s0._pocketLastSentAt >= FOLLOW_THROTTLE_MS &&
        (s0._pocketLastSentBpm === null ||
          Math.abs(tempo - s0._pocketLastSentBpm) >= FOLLOW_MIN_DELTA_BPM)
      ) {
        void invoke("set_pocket_tempo", { tempoBpm: tempo });
        set({
          _pocketLastSentBpm: tempo,
          _pocketLastSentAt: now,
          pocketTempo: tempo,
        });
      }
    }
    return set((s) => {
      if (!s.listenToRoom || !perception) {
        return { perception };
      }
      const next = pushLane(s.chordLane, s.laneRinging, perception);
      return {
        perception,
        chordLane: next.lane,
        laneRinging: next.ringing,
      };
    });
  },

  setListenToRoom: (on) =>
    set((s) =>
      on
        ? {
            listenToRoom: true,
            // First enable marks where "the room" begins for the recap
            // sketch; re-enables keep the original mark.
            jamStartSecs: s.jamStartSecs ?? s.elapsedSecs,
          }
        : { listenToRoom: false, chordLane: [], laneRinging: false },
    ),

  exploreChord: async (rootPc, quality) => {
    if (get().status !== "listening") {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("explore_chord", {
        rootPc,
        quality,
      });
      set({ explore: dto, exploreNotice: null });
    } catch (err) {
      // The backend's message is written for the player — show it.
      set({ exploreNotice: String(err) });
    }
  },

  setAccompanimentKey: async (tonic, minor) => {
    const prev = { keyPinned: get().keyPinned, pinnedKey: get().pinnedKey };
    // Optimistic: the chip reflects the pin (and shows the pinned key, not the
    // still-changing auto-read) immediately; the backend applies it to the live
    // band (or stores it for the next band start).
    set({ keyPinned: true, pinnedKey: keyOptionFor(tonic, minor) });
    try {
      await invoke("set_accompaniment_key", { tonic, minor });
    } catch (err) {
      console.error("set_accompaniment_key failed:", err);
      set(prev); // roll back so the UI doesn't lie about a pin that didn't take
    }
  },

  // "Lock" simply pins whatever key is currently shown — a concrete key, so it
  // survives a band restart and the panel can display it honestly.
  lockAccompanimentKey: async () => {
    const key = get().perception?.key;
    if (!key) return;
    await get().setAccompanimentKey(key.tonic, key.mode === "minor");
  },

  clearAccompanimentKey: async () => {
    const prev = { keyPinned: get().keyPinned, pinnedKey: get().pinnedKey };
    set({ keyPinned: false, pinnedKey: null });
    try {
      await invoke("clear_accompaniment_key");
    } catch (err) {
      console.error("clear_accompaniment_key failed:", err);
      set(prev);
    }
  },

  switchInstrument: async (name: string, vibratoToleranceCents = 15.0) => {
    const { status } = get();
    if (status !== "listening") {
      throw new Error(
        `cannot switch instrument from status=${status} — start a session first`,
      );
    }
    const segmentId = await invoke<string>("switch_instrument", {
      instrument: name,
      practiceMode: get().practiceMode,
    });
    set({ instrumentName: name, segmentId });
    useAudioStore.getState().setInstrument(name, vibratoToleranceCents);
  },

  pushPhrase: (phrase) =>
    set((state) => ({ phrases: [...state.phrases, phrase] })),

  pushTip: (tip, phraseIndex) =>
    set((state) => ({
      tipQueue: [
        ...state.tipQueue,
        { id: newId(), tip, receivedAt: Date.now(), phraseIndex },
      ],
    })),

  requestCoachingTip: async (phrase) => {
    const { coachingEnabled, status, elapsedSecs, phrases } = get();
    // Opt-in gate: no IPC at all when the user hasn't enabled online coaching,
    // and only while a session is live.
    if (!coachingEnabled || status !== "listening") {
      return;
    }
    try {
      const tip = await invoke<CoachingTip | null>("get_coaching_tip", {
        phrase,
        sessionDurationSecs: elapsedSecs,
        phrasesPlayed: phrases.length,
      });
      // `null` is the honest "no tip" signal (offline / rate-limited / API
      // failure). Surface nothing — the panel shows its empty state.
      if (!tip) {
        return;
      }
      get().pushTip(tip, phrase.phrase_index);
      // Persist it into the session recorder so it lands in history + recap.
      // A failure here must not break the live loop — log and move on.
      try {
        await invoke("record_coaching_tip", {
          phraseIndex: phrase.phrase_index,
          tip,
        });
      } catch (err) {
        console.error("Failed to persist coaching tip:", err);
      }
    } catch (err) {
      // The live tip is best-effort; never let a failed request disrupt
      // the session.
      console.error("Failed to fetch coaching tip:", err);
    }
  },

  dismissTip: (id) =>
    set((state) => ({
      tipQueue: state.tipQueue.filter((q) => q.id !== id),
    })),

  // Replace rather than append: the card shows one reveal at a time and a new
  // reveal *supersedes* the old (#253 AC7 "replaces, does not stack"). Appending
  // would let a still-queued older reveal resurface once the newer one's
  // dismiss timer fires — so we keep the queue at a single, latest entry.
  pushReveal: (reveal, phraseIndex) =>
    set({
      revealQueue: [
        { id: newId(), reveal, receivedAt: Date.now(), phraseIndex },
      ],
    }),

  requestReveal: async (phrase) => {
    const { status, perception } = get();
    // Only during a live session, and only when we actually hear a key — the
    // reveal is about the music the player is in right now.
    if (status !== "listening") {
      return;
    }
    const key = perception?.key;
    if (!key) {
      return;
    }
    try {
      // Curated + offline in slice 1: the backend makes no network call and
      // returns `null` when confidence is low, the mode is uncurated, or this
      // phrase isn't on the reveal cadence. Honor the silence.
      const reveal = await invoke<Reveal | null>("get_reveal", {
        tonic: key.tonic,
        mode: key.mode,
        confidence: key.confidence,
        phraseIndex: phrase.phrase_index,
      });
      if (!reveal) {
        return;
      }
      get().pushReveal(reveal, phrase.phrase_index);
      // Persist the unlock into the Learner Model collection (#253 S3). The
      // backend dedups (a repeat doesn't grow the count) and returns the new
      // distinct-collection size for the card's little counter. A failure here
      // must never break the live loop — log and move on.
      try {
        const count = await invoke<number>("record_reveal", {
          concept: reveal.concept,
          connection: reveal.connection,
        });
        set({ collectionCount: count });
      } catch (err) {
        console.error("Failed to record reveal:", err);
      }
    } catch (err) {
      // Best-effort: a failed reveal must never disrupt the session.
      console.error("Failed to fetch reveal:", err);
    }
  },

  dismissReveal: (id) =>
    set((state) => ({
      revealQueue: state.revealQueue.filter((q) => q.id !== id),
    })),

  startLesson: async () => {
    if (get().status !== "listening") {
      throw new Error("start a practice session before starting a lesson");
    }
    const step = await invoke<LessonStepDto>("start_lesson", {});
    set({
      lessonDrill: step.drill,
      lessonScore: null,
      lessonRecap: null,
      lessonNotice: null,
    });
  },

  submitDrill: async () => {
    // Double-tap guard: a second submit while one is in flight would grade the
    // NEXT drill against an empty take and silently skip it (#254 review M2).
    if (!get().lessonDrill || get().lessonSubmitting) {
      return;
    }
    set({ lessonSubmitting: true });
    try {
      const step = await invoke<LessonStepDto>("submit_drill", {});
      set({
        lessonDrill: step.drill,
        lessonScore: step.score,
        lessonRecap: step.recap,
        lessonNotice: null,
      });
    } catch (err) {
      // Keep the drill on screen for a retry, and SHOW the backend's calm
      // message (e.g. the eager-tap "I didn't catch that yet…") — a silent
      // blink is worse UX than the old dishonest 0%.
      console.error("submit_drill failed:", err);
      set({ lessonNotice: String(err) });
    } finally {
      set({ lessonSubmitting: false });
    }
  },

  endLesson: async () => {
    set({
      lessonDrill: null,
      lessonScore: null,
      lessonRecap: null,
      lessonNotice: null,
    });
    try {
      await invoke("end_lesson", {});
    } catch (err) {
      console.error("end_lesson failed:", err);
    }
  },

  clearLessonRecap: () => set({ lessonRecap: null, lessonScore: null }),

  startExplore: async (tonic, mode) => {
    if (get().status !== "listening") {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("start_explore_variation", {
        tonic,
        mode,
      });
      set({ explore: dto });
    } catch (err) {
      console.error("start_explore_variation failed:", err);
    }
  },

  exploreMeasure: async (measureNumber: number) => {
    const scoreId = get().activeScore?.id;
    if (!scoreId) {
      // Edge: the recap outlived its score reference — say so instead of a
      // dead button (S3/S5 review finding 9).
      set({
        bridgeNotice: "that score isn't open anymore — re-import it first",
      });
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("explore_measure", {
        scoreId,
        measureNumber,
      });
      // Park it for the next session, then head to the selector — the
      // session screens clear `explore`, so the handoff uses its own slot.
      set({ pendingExplore: dto, bridgeNotice: null });
      get().returnToSelector();
    } catch (err) {
      // A calm refusal must not vaporize the recap (S5 review MUST-FIX 3).
      set({ bridgeNotice: String(err) });
    }
  },

  addOpenerItem: async (item) => {
    const items = [...get().openerItems, item];
    set({ openerItems: items });
    await get()._refreshOpenerPreview(items);
  },

  removeOpenerItem: async (index) => {
    const items = get().openerItems.filter((_, i) => i !== index);
    set({ openerItems: items });
    await get()._refreshOpenerPreview(items);
  },

  clearOpener: () =>
    set((s) => ({
      openerItems: [],
      openerPreview: null,
      openerNotice: null,
      openerTonic: null,
      openerDirection: "forward",
      openerPreviewedDirection: "forward",
      // Round-3 review MF1: a reset INVALIDATES in-flight refreshes —
      // otherwise a late response repaints a ghost preview (and a stale
      // tonic) onto the just-cleared builder.
      _openerRefreshSeq: s._openerRefreshSeq + 1,
    })),

  setOpenerDirection: (direction) => {
    set({ openerDirection: direction });
    // A direction change re-voices the preview immediately.
    void get()._refreshOpenerPreview(get().openerItems);
  },

  /** Internal: recompute the pure preview after any item change. */
  _refreshOpenerPreview: async (items: StarterItem[]) => {
    // S2b review MF2: a MONOTONIC token — the S1 items-identity guard
    // couldn't tell two direction-triggered refreshes apart (same array).
    // Only the latest refresh commits anything: dto, tonic, direction.
    const seq = get()._openerRefreshSeq + 1;
    set({ _openerRefreshSeq: seq });
    if (items.length === 0) {
      set({ openerPreview: null, openerNotice: null });
      return;
    }
    // #419 S2b: read the live key ONCE per refresh — confident reads only
    // (the shared assert threshold, KEY_ASSERT_CONFIDENCE).
    const key = get().perception?.key;
    const tonic =
      key && key.confidence >= KEY_ASSERT_CONFIDENCE ? key.tonic : null;
    const direction = get().openerDirection;
    try {
      const dto = await invoke<ExploreDto>("preview_opener", {
        items,
        tonic,
        direction,
      });
      if (get()._openerRefreshSeq !== seq) {
        return; // A newer refresh owns the surface now.
      }
      // Tonic and direction commit WITH the dto they built — Begin sends
      // exactly the pair the player is looking at.
      set({
        openerPreview: dto,
        openerNotice: null,
        openerTonic: tonic,
        openerPreviewedDirection: direction,
      });
    } catch (err) {
      if (get()._openerRefreshSeq !== seq) {
        return;
      }
      set({ openerPreview: null, openerNotice: String(err) });
    }
  },

  beginOpener: async () => {
    if (get().status !== "listening") {
      return;
    }
    const items = get().openerItems;
    if (items.length === 0) {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("begin_opener", {
        items,
        // The pair the last ACCEPTED preview committed — never a fresh
        // read, never a setting a mid-flight refresh hasn't painted.
        tonic: get().openerTonic,
        direction: get().openerPreviewedDirection,
      });
      // The opener becomes the session's exploration — the same surface a
      // lifted lick lands on — and the builder resets for next time.
      set((s) => ({
        explore: dto,
        exploreNotice: null,
        openerItems: [],
        openerPreview: null,
        openerNotice: null,
        openerTonic: null,
        openerDirection: "forward",
        openerPreviewedDirection: "forward",
        // Round-3 review MF1: Begin's reset kills in-flight refreshes.
        _openerRefreshSeq: s._openerRefreshSeq + 1,
      }));
    } catch (err) {
      set({ openerNotice: String(err) });
    }
  },

  applyOpenerRecipe: async (items, direction) => {
    set({ openerItems: items, openerDirection: direction });
    await get()._refreshOpenerPreview(items);
  },

  beginOpenerRecall: async () => {
    if (get().status !== "listening") {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("begin_opener_recall");
      // Same landing as beginOpener: the recalled opener becomes the
      // session's exploration and the builder resets.
      set((s) => ({
        explore: dto,
        exploreNotice: null,
        openerItems: [],
        openerPreview: null,
        openerNotice: null,
        openerTonic: null,
        openerDirection: "forward",
        openerPreviewedDirection: "forward",
        _openerRefreshSeq: s._openerRefreshSeq + 1,
      }));
    } catch (err) {
      set({ openerNotice: String(err) });
    }
  },

  exploreLastPhrase: async () => {
    if (get().status !== "listening") {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("explore_last_phrase", {});
      set({ explore: dto, exploreNotice: null });
    } catch (err) {
      // The backend's message is written for the player — show it.
      set({ exploreNotice: String(err) });
    }
  },

  exploreProgression: async () => {
    if (get().status !== "listening") {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("explore_progression", {});
      set({ explore: dto, exploreNotice: null });
    } catch (err) {
      // The backend's message is written for the player — show it.
      set({ exploreNotice: String(err) });
    }
  },

  exploreMeasureLive: async (measureNumber) => {
    if (get().status !== "listening") {
      return;
    }
    const scoreId = get().activeScore?.id;
    if (!scoreId) {
      return; // no score, no overlay — belt and braces
    }
    try {
      const dto = await invoke<ExploreDto>("explore_measure", {
        scoreId,
        measureNumber,
      });
      // The exploration takes the stage IN the running session — same view
      // swap as "work on my last lick"; "Back to listening" returns.
      set({ explore: dto, exploreNotice: null });
    } catch (err) {
      // Calm refusal (rest-only measure, too busy, …) — written for the
      // player by the backend; show it without leaving the score.
      set({ exploreNotice: String(err) });
    }
  },

  applyChip: async (delta) => {
    if (!get().explore) {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("apply_variation_delta", { delta });
      set({ explore: dto, exploreNotice: null });
    } catch (err) {
      // A failed mutation keeps the current rep on screen — visibly.
      set({ exploreNotice: String(err) });
    }
  },

  editExploreNote: async (index, edit) => {
    if (!get().explore) {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("edit_explore_note", {
        index,
        edit,
      });
      set({ explore: dto, exploreNotice: null });
    } catch (err) {
      // A refused edit keeps the current rep — and TELLS the player why
      // ("that's as far as this note can go"), never a silent dead tap.
      set({ exploreNotice: String(err) });
    }
  },

  undoExploreEdit: async () => {
    if (!get().explore) {
      return;
    }
    try {
      const dto = await invoke<ExploreDto>("undo_explore_edit", {});
      set({ explore: dto, exploreNotice: null });
    } catch (err) {
      set({ exploreNotice: String(err) });
    }
  },

  endExplore: async () => {
    set({ explore: null, exploreNotice: null });
    try {
      await invoke("end_explore", {});
    } catch (err) {
      console.error("end_explore failed:", err);
    }
  },

  setCursorPosition: (pos) => set({ cursorPosition: pos }),

  tick: () => {
    const { status, startedAtMs } = get();
    if (status !== "listening" || startedAtMs === null) {
      return;
    }
    // Integer seconds from start. Rendering MM:SS from this derived
    // display string is trivial and re-renders at most 1Hz.
    set({ elapsedSecs: Math.floor((Date.now() - startedAtMs) / 1000) });
  },

  returnToSelector: () =>
    set({
      screen: "selector",
      bridgeNotice: null,
      status: "idle",
      recap: null,
      recapError: null,
      sessionId: null,
      instrumentName: null,
      segmentId: null,
      startedAtMs: null,
      elapsedSecs: 0,
      phrases: [],
      tipQueue: [],
      revealQueue: [],
      lessonDrill: null,
      lessonScore: null,
      lessonRecap: null,
      lessonSubmitting: false,
      lessonNotice: null,
      explore: null,
      exploreNotice: null,
      activeScore: null,
      activeScoreXml: null,
      cursorPosition: null,
      accompanimentPlaying: false,
      perception: null,
      keyPinned: false,
      pinnedKey: null,
    }),

  setCoachingEnabled: (on) => {
    saveCoachingPref(on);
    set({ coachingEnabled: on });
  },

  setPracticeMode: (mode) => {
    savePracticeModePref(mode);
    set({ practiceMode: mode });
  },

  goToHistory: () =>
    set({
      screen: "history",
    }),

  goToConnections: () =>
    set({
      screen: "connections",
    }),
}));
