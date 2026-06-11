//! Session recorder — accumulates phrase summaries and coaching tips
//! during a practice session, then delegates to a [`RecapGenerator`] to
//! produce a teacher-quality [`SessionRecap`].
//!
//! This module runs on the processing thread, NOT the audio thread, so
//! heap allocation (Vec, String, etc.) is allowed.
//!
//! # Data model (Story #14, PR 0)
//!
//! A session is a **tree**: `Session → Vec<Participant> → Vec<InstrumentSegment>`.
//! A `Participant` represents one human/input (`display_name` + `input_source`);
//! an `InstrumentSegment` represents a contiguous span during which that
//! participant was playing a single instrument. The solo MVP only ever
//! populates one participant with one or more segments (one per instrument
//! switch). The tree exists now so the ensemble mode story (#40) and
//! multi-instrumentalist switching don't need a second data-model migration.
//! See `docs/design/decisions-log.md` entry "Participants + segments data
//! model, even in solo MVP" for the full rationale.
//!
//! # Decoupling from the coaching module
//!
//! The recap is generated through the [`RecapGenerator`] trait rather
//! than by calling the coaching module directly. That keeps this module
//! compilable ahead of the coaching LLM engine landing on `main`: tests
//! use a mock implementation, and production wiring (Story #14, free
//! play) plugs in the real coaching engine.

use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::fingerprint::MusicalFingerprint;
use crate::idiom_recap::IdiomMatch;
use crate::phrase::PhraseSummary;
use crate::store::TasteProfile;

// ---------------------------------------------------------------------------
// Practice mode
// ---------------------------------------------------------------------------

/// Coaching behavior mode during a practice session.
///
/// Different modes adjust how the AI coaches:
/// - **Warmup:** Monitors silently, only suggests readiness indicators.
/// - **Practice:** Full coaching with phrase-level feedback.
/// - **RunThrough:** Silent during performance, recap only at end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PracticeMode {
    /// Monitor silently, comment only on readiness (tone stabilizing, range).
    Warmup,
    /// Full coaching — phrase-level feedback, tips, technique suggestions.
    #[default]
    Practice,
    /// Stay silent during performance, full recap at end.
    RunThrough,
}

impl PracticeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::Practice => "practice",
            Self::RunThrough => "run_through",
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while running a practice session.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The underlying [`RecapGenerator`] returned an error.
    #[error("recap generator failed: {0}")]
    RecapFailed(String),
    /// The session ended without a single recorded phrase. We refuse to
    /// synthesize a recap out of nothing, because that would mask a bug
    /// in the caller (e.g. pushing phrases to the wrong recorder).
    #[error("no phrases recorded — cannot generate recap")]
    Empty,
    /// A recorder operation required a currently-open [`InstrumentSegment`]
    /// but none was open. Should never happen in the MVP flow — defensive.
    #[error("no instrument segment is currently open")]
    NoOpenSegment,
    /// A recorder operation required at least one [`Participant`] but
    /// none was present. Only reachable by a recorder constructed via
    /// `new()` that was later mutated by internal code — defensive.
    #[error("recorder has no participant")]
    NoParticipant,
}

// ---------------------------------------------------------------------------
// Id newtypes
// ---------------------------------------------------------------------------

/// Generate the standard id newtype impls (Display, FromStr, Default,
/// Debug, Copy, PartialEq, Eq, Hash, Serialize, Deserialize).
macro_rules! uuid_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a fresh random id.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Hyphenated string representation.
            pub fn as_str(&self) -> String {
                self.0.to_string()
            }

            /// The inner UUID, if a caller needs it.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

uuid_newtype!(SessionId, "Unique identifier for a practice session.");
uuid_newtype!(
    ParticipantId,
    "Stable id for a participant within one session."
);
uuid_newtype!(
    SegmentId,
    "Stable id for an instrument segment within one session."
);
uuid_newtype!(
    ScoreId,
    "Unique identifier for a loaded score in the library."
);

// ---------------------------------------------------------------------------
// Recorded tip
// ---------------------------------------------------------------------------

/// A coaching tip captured against a specific phrase during a session.
///
/// `severity` and `category` are stored as strings so this module does
/// not need to depend on the coaching module's enum types. The
/// coaching enums serialize in snake_case (e.g. `"suggestion"`), so
/// callers should pass the same lower-case string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedTip {
    /// Index of the phrase within the containing [`InstrumentSegment`]
    /// that this tip was generated for.
    pub phrase_index: usize,
    /// Human-readable coaching text.
    pub text: String,
    /// Severity string (e.g. `"encouragement"`, `"suggestion"`, `"focus"`).
    pub severity: String,
    /// Category string (e.g. `"tone"`, `"intonation"`, ...).
    pub category: String,
    /// When the tip was recorded.
    pub recorded_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Participants & instrument segments
// ---------------------------------------------------------------------------

/// Where a participant's audio is coming from.
///
/// MVP supports only the default system mic; multi-channel /
/// audio-interface-channel / remote inputs are future work. See
/// `docs/design/decisions-log.md` for the rejected alternatives
/// (networked peers, per-player mics) and why ensemble mode uses a
/// single shared mic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputSource {
    /// The OS default microphone input. Only option in the solo MVP.
    DefaultMic,
    // AudioInterfaceChannel(u8) — reserved for ensemble mode (#40)
    // Networked { peer_id: String } — REJECTED, see decisions-log.md
}

/// A contiguous span within a session during which one participant was
/// playing a single instrument. Segments are opened implicitly (by
/// [`SessionRecorder::new`]) or explicitly (by [`SessionRecorder::switch_instrument`])
/// and closed when a newer segment starts or the session is completed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentSegment {
    pub id: SegmentId,
    /// Profile name, e.g. `"trumpet"` or `"piano"`.
    pub instrument: String,
    pub started_at: DateTime<Utc>,
    /// `None` while the segment is still open. Set when a newer segment
    /// starts or the session ends.
    pub ended_at: Option<DateTime<Utc>>,
    /// Practice mode (Warmup, Practice, RunThrough).
    pub practice_mode: PracticeMode,
    /// Phrases recorded while this segment was open.
    pub phrases: Vec<PhraseSummary>,
    /// Coaching tips recorded while this segment was open.
    pub tips: Vec<RecordedTip>,
}

/// A participant in a session (one human/input). The solo MVP always
/// has exactly one participant with `display_name: "You"` and
/// `input_source: InputSource::DefaultMic`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Participant {
    pub id: ParticipantId,
    /// `"You"` in the solo MVP. In ensemble mode (#40) this becomes the
    /// section or player label (e.g. `"Trumpets"`, `"Alice"`).
    pub display_name: String,
    pub input_source: InputSource,
    /// Ordered list of instrument segments. Always at least one in the
    /// MVP flow — [`SessionRecorder::new`] opens the first segment
    /// immediately.
    pub segments: Vec<InstrumentSegment>,
}

// ---------------------------------------------------------------------------
// Recap types
// ---------------------------------------------------------------------------

/// Input passed to a [`RecapGenerator`] to produce a [`SessionRecap`].
///
/// The participants tree is flattened into `phrases` and `tips` before
/// being handed to the LLM — the prompt still receives the cross-segment
/// aggregate, so LLM-side changes aren't required for this PR. Future
/// prompt work can add segment-aware context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapInput {
    /// Primary instrument for the session. In a multi-segment session
    /// this is the instrument of the first segment (lossy — see
    /// `docs/design/story-14-free-play-mode.md`).
    pub instrument: String,
    /// Total session duration in seconds.
    pub duration_secs: f64,
    /// Practice mode during the session (affects coaching style).
    pub practice_mode: PracticeMode,
    /// All phrase summaries recorded during the session, flattened
    /// across all segments in segment order.
    pub phrases: Vec<PhraseSummary>,
    /// All coaching tips recorded during the session, flattened across
    /// all segments in segment order. Note: `phrase_index` is relative
    /// to the segment it was recorded in, not the flattened list.
    pub tips: Vec<RecordedTip>,
    /// Title of the score the student practised with, if any. Lets the
    /// recap name the piece ("your work on the Haydn") and lean on the
    /// per-phrase `score_position` measure numbers. `None` in free play.
    pub score_title: Option<String>,
    /// Confidence-gated idiom matches for the session, computed **offline**
    /// on-device from the captured audio (see [`crate::idiom_recap`]). Each is
    /// a grounded audio-similarity proximity ("reminds me of"), never an
    /// asserted fact. Empty when nothing cleared the engine's gate — the recap
    /// then stays silent on idiom. `serde(default)` so older inputs still load.
    #[serde(default)]
    pub idiom_notes: Vec<IdiomMatch>,
    /// The student's stated [`TasteProfile`] — genres, artists, goals,
    /// experience. This is the *only* place the measured fingerprint (facts)
    /// and the stated preferences join: at coaching time, so the coach can
    /// relate what was played to the music in the student's world (see
    /// `docs/architecture/platform-spine-personalization.md`). `None` at cold
    /// start (no profile captured yet), in which case the coach falls back to
    /// its existing genre-neutral behavior — silence over a hollow connection.
    #[serde(default)]
    pub taste_profile: Option<TasteProfile>,
}

/// The post-session recap shown to the student.
///
/// Rendered from a carefully constructed LLM prompt (see the coaching
/// module). All text fields are teacher-quality natural language:
/// no letter grades, no percentages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRecap {
    /// Overall qualitative assessment of the session.
    pub overall_assessment: String,
    /// 2–4 specific strengths the student demonstrated.
    pub strengths: Vec<String>,
    /// 2–4 concrete areas to improve.
    pub areas_to_improve: Vec<String>,
    /// Suggestions for what to focus on in the next session.
    pub next_session_suggestions: Vec<String>,
    /// Total session duration in seconds (copied from `RecapInput`).
    pub duration_secs: f64,
    /// Number of phrases played (copied from `RecapInput.phrases.len()`).
    pub phrase_count: usize,
    /// Instrument played (first-segment instrument for multi-segment
    /// sessions — copied from `RecapInput.instrument`).
    pub instrument: String,
    /// The session's [`MusicalFingerprint`] — the unified read of what we
    /// measured about this session's musicianship (tone, key, intonation,
    /// groove). This replaces the four scattered `session_*` fields: one
    /// representation, not four parallel ones. Persisted in the recap JSON for
    /// trend tracking ("warmer than last time") and the teacher feed, and
    /// consumed by the upcoming personalization / cultural-relevance engine.
    ///
    /// `None` when nothing was measured (every dimension's evidence gate
    /// failed). When `Some`, individual dimensions inside it are still
    /// `Option`: each is present only when its gate passed. Additive +
    /// `serde(default)` so recaps saved before the fingerprint existed still
    /// load (defaulting to `None`).
    #[serde(default)]
    pub fingerprint: Option<MusicalFingerprint>,
    /// Confidence-gated idiom flavours for the session — grounded, hedged
    /// "reminds me of" notes computed **fully offline** on-device from the
    /// captured audio (see [`crate::idiom_recap`]). Each [`IdiomMatch`] is a
    /// real audio-similarity proximity, never an asserted fact; the UI surfaces
    /// them quietly and only when present. Empty when nothing cleared the
    /// engine's confidence gate ("silence > lies"). Additive + `serde(default)`
    /// so recaps saved before idiom landed still load (defaulting to empty).
    #[serde(default)]
    pub idiom_notes: Vec<IdiomMatch>,
    /// Grounded, hedged cross-genre **connections** relating what the student
    /// actually played (the measured `fingerprint`) to the genres/artists in
    /// their stated `TasteProfile` — the Phase 4 "coach, don't judge" cultural
    /// bridge. Each entry is style/feel/technique-level only ("the way you're
    /// shaping that line reminds me of how horn players phrase in soul"), never
    /// a hard claim about a specific recording.
    ///
    /// Populated **only** when a taste profile existed *and* the session
    /// produced enough measured signal to ground a connection honestly *and*
    /// the model actually returned one. Empty otherwise — at cold start, on a
    /// thin signal, or when the coach chose silence over a hollow link. The
    /// data model stays honest about when it's empty: `Vec::is_empty()` means
    /// "no grounded connection", not "feature off". Additive + `serde(default)`
    /// so recaps saved before connections existed still load.
    #[serde(default)]
    pub connections: Vec<String>,
}

// ---------------------------------------------------------------------------
// RecapGenerator trait
// ---------------------------------------------------------------------------

/// Abstraction over "turn a [`RecapInput`] into a [`SessionRecap`]".
///
/// Production implementations will call an LLM. Tests use a canned
/// mock. Using a trait here decouples [`SessionRecorder`] from the
/// coaching module so these can land as independent PRs.
#[async_trait]
pub trait RecapGenerator: Send + Sync {
    /// Produce a recap from accumulated session data.
    async fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError>;

    /// Mirror the offline-first airplane switch onto this generator's
    /// underlying engine, if it has one. The default is a no-op: a generator
    /// that never touches the network (mocks, on-device fallbacks) ignores it.
    /// A networked generator (the LLM recap path) overrides this so that an
    /// `Offline` policy makes an outbound recap request structurally impossible.
    ///
    /// Named distinctly from the inherent `CoachingEngine::set_network_policy`
    /// so the two never collide in method resolution (`CoachingEngine` is itself
    /// a `RecapGenerator`).
    ///
    /// See `docs/architecture/offline-first-and-network-transparency.md`.
    async fn apply_network_policy(&self, _policy: crate::coaching::NetworkPolicy) {}
}

// ---------------------------------------------------------------------------
// CompletedSession
// ---------------------------------------------------------------------------

/// A finalised practice session, with authoritative timestamps and all
/// recorded data. Produced by [`SessionRecorder::complete`].
///
/// This is the unit of persistence: once you have a `CompletedSession`,
/// you can store it, generate a recap from it (once or multiple times),
/// or discard it — independent of whether the LLM recap call succeeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedSession {
    pub id: SessionId,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_secs: f64,
    /// The full participants tree. Always len >= 1; MVP only ever
    /// populates one.
    pub participants: Vec<Participant>,
    /// Title of the score practised with, if any. Carried through to the
    /// recap so the LLM can reference the piece by name. `None` in free play.
    pub score_title: Option<String>,
}

impl CompletedSession {
    /// The primary instrument — instrument of the first segment of the
    /// first participant. Panics if the session has no participants or
    /// no segments, which [`SessionRecorder::complete`] guarantees.
    pub fn primary_instrument(&self) -> &str {
        &self.participants[0].segments[0].instrument
    }

    /// Total number of phrases across all segments and participants.
    pub fn phrase_count(&self) -> usize {
        self.participants
            .iter()
            .flat_map(|p| p.segments.iter())
            .map(|s| s.phrases.len())
            .sum()
    }

    /// Flattened vector of every phrase recorded, in segment order.
    pub fn all_phrases(&self) -> Vec<PhraseSummary> {
        self.participants
            .iter()
            .flat_map(|p| p.segments.iter().flat_map(|s| s.phrases.iter().cloned()))
            .collect()
    }

    /// Flattened vector of every tip recorded, in segment order.
    pub fn all_tips(&self) -> Vec<RecordedTip> {
        self.participants
            .iter()
            .flat_map(|p| p.segments.iter().flat_map(|s| s.tips.iter().cloned()))
            .collect()
    }

    /// Build a [`RecapInput`] from this completed session. The LLM
    /// prompt still receives flat phrases + tips, so we flatten the
    /// participants tree here. `instrument` is the primary instrument
    /// (first segment) and `practice_mode` is from the first segment as well.
    pub fn to_recap_input(&self) -> RecapInput {
        self.to_recap_input_with_idioms(Vec::new())
    }

    /// Like [`Self::to_recap_input`], but attaches `idiom_notes` — the
    /// confidence-gated, **offline** idiom matches computed at the session
    /// boundary from the captured audio. Audio itself isn't part of a
    /// persisted `CompletedSession`, so idiom analysis runs in the live
    /// recap-building path (see the Tauri shell) and is threaded in here.
    pub fn to_recap_input_with_idioms(&self, idiom_notes: Vec<IdiomMatch>) -> RecapInput {
        let primary_segment = &self.participants[0].segments[0];
        RecapInput {
            instrument: primary_segment.instrument.clone(),
            duration_secs: self.duration_secs,
            practice_mode: primary_segment.practice_mode,
            phrases: self.all_phrases(),
            tips: self.all_tips(),
            score_title: self.score_title.clone(),
            idiom_notes,
            // The taste profile is owned by the persistence layer, not the
            // recorder — it's read from the store and joined to the recap input
            // at coaching time (see `generate_recap_with_context`). A bare
            // `CompletedSession` carries no profile, so default to `None` here.
            taste_profile: None,
        }
    }

    /// Like [`Self::generate_recap`], but joins the student's stated
    /// [`TasteProfile`] to the recap input first, so the coach can relate the
    /// measured musicianship to the genres/artists in the student's world.
    ///
    /// This is the *only* place the measured fingerprint (facts) and the stated
    /// preferences (context) meet, per the personalization spine: the recorder
    /// stays profile-agnostic, and the command layer reads the profile from the
    /// store and threads it in here. `profile == None` (cold start) is
    /// equivalent to [`Self::generate_recap`] — no forced cross-genre framing.
    ///
    /// The same authoritative-fields guarantee as [`Self::generate_recap`]
    /// applies: `duration_secs`, `phrase_count`, and `instrument` are
    /// overwritten from this session regardless of what the generator emits.
    pub async fn generate_recap_with_profile(
        &self,
        generator: &dyn RecapGenerator,
        profile: Option<TasteProfile>,
    ) -> Result<SessionRecap, SessionError> {
        self.generate_recap_with_context(generator, profile, Vec::new())
            .await
    }

    /// Generate a recap via the given [`RecapGenerator`].
    ///
    /// **Authoritative-fields guarantee:** regardless of what the
    /// generator emits, the returned recap's `duration_secs`,
    /// `phrase_count`, and `instrument` are overwritten with the values
    /// from this [`CompletedSession`]. An LLM can hallucinate or
    /// miscalculate these; the recorder is the source of truth.
    pub async fn generate_recap(
        &self,
        generator: &dyn RecapGenerator,
    ) -> Result<SessionRecap, SessionError> {
        self.generate_recap_with_context(generator, None, Vec::new())
            .await
    }

    /// Like [`Self::generate_recap`], but feeds the generator the
    /// confidence-gated, **offline** `idiom_notes` for this session so the
    /// recap can surface grounded idiom flavours.
    pub async fn generate_recap_with_idioms(
        &self,
        generator: &dyn RecapGenerator,
        idiom_notes: Vec<IdiomMatch>,
    ) -> Result<SessionRecap, SessionError> {
        self.generate_recap_with_context(generator, None, idiom_notes)
            .await
    }

    /// The combined recap-generation path that threads **both** personalization
    /// context (`profile`, #166) and the **offline** `idiom_notes` (#168) into a
    /// single [`RecapInput`]. The two are complementary and independent: idiom
    /// notes are grounded, confidence-gated audio proximities that surface even
    /// offline, while connections are the LLM-hedged cross-genre lines that
    /// depend on a taste profile. The other `generate_recap*` methods are thin
    /// wrappers that default one or both of these to "absent".
    ///
    /// The same authoritative-fields guarantee as [`Self::generate_recap`]
    /// applies: `duration_secs`, `phrase_count`, and `instrument` are
    /// overwritten from this session regardless of what the generator emits.
    pub async fn generate_recap_with_context(
        &self,
        generator: &dyn RecapGenerator,
        profile: Option<TasteProfile>,
        idiom_notes: Vec<IdiomMatch>,
    ) -> Result<SessionRecap, SessionError> {
        let mut input = self.to_recap_input_with_idioms(idiom_notes);
        input.taste_profile = profile;
        let mut recap = generator.generate_recap(&input).await?;
        recap.duration_secs = self.duration_secs;
        recap.phrase_count = self.phrase_count();
        recap.instrument = self.primary_instrument().to_owned();
        Ok(recap)
    }
}

// ---------------------------------------------------------------------------
// SessionRecorder
// ---------------------------------------------------------------------------

/// Records a single practice session: phrases, coaching tips, and
/// wall-clock timing, across one or more [`InstrumentSegment`]s owned
/// by a single [`Participant`] (in the MVP).
///
/// A recorder does NOT own a [`RecapGenerator`]; recap generation is a
/// separate step on the returned [`CompletedSession`]. This keeps
/// session data safe from LLM failures and lets callers choose whether
/// to generate a recap at all.
pub struct SessionRecorder {
    session_id: SessionId,
    started_at: DateTime<Utc>,
    participants: Vec<Participant>,
    /// Title of the score being practised, if the session is score-backed.
    /// Set once at session start via [`set_score_title`](Self::set_score_title).
    score_title: Option<String>,
}

impl SessionRecorder {
    /// Create a new recorder for the given instrument and practice mode.
    /// The start time is captured as `Utc::now()` at construction, and the
    /// first [`InstrumentSegment`] is opened immediately.
    pub fn new(initial_instrument: String, practice_mode: PracticeMode) -> Self {
        let started_at = Utc::now();
        let first_segment = InstrumentSegment {
            id: SegmentId::new(),
            instrument: initial_instrument,
            started_at,
            ended_at: None,
            practice_mode,
            phrases: Vec::new(),
            tips: Vec::new(),
        };
        let participant = Participant {
            id: ParticipantId::new(),
            display_name: "You".to_owned(),
            input_source: InputSource::DefaultMic,
            segments: vec![first_segment],
        };
        Self {
            session_id: SessionId::new(),
            started_at,
            participants: vec![participant],
            score_title: None,
        }
    }

    /// Record the title of the score this session is practising with, so
    /// the end-of-session recap can name the piece. Call once at session
    /// start; a no-op-equivalent `None` leaves the session in free-play
    /// framing.
    pub fn set_score_title(&mut self, title: Option<String>) {
        self.score_title = title;
    }

    /// Mutable access to the sole participant (MVP invariant: exactly one).
    fn sole_participant_mut(&mut self) -> Result<&mut Participant, SessionError> {
        self.participants
            .first_mut()
            .ok_or(SessionError::NoParticipant)
    }

    /// Mutable access to the currently-open (trailing) segment of the
    /// sole participant. Errors if no segment is open — which shouldn't
    /// happen post-`new()` but we check defensively.
    fn current_segment_mut(&mut self) -> Result<&mut InstrumentSegment, SessionError> {
        let participant = self.sole_participant_mut()?;
        let last = participant
            .segments
            .last_mut()
            .ok_or(SessionError::NoOpenSegment)?;
        if last.ended_at.is_some() {
            return Err(SessionError::NoOpenSegment);
        }
        Ok(last)
    }

    /// Append a completed phrase to the *currently open segment* of the
    /// *sole participant*.
    pub fn record_phrase(&mut self, phrase: PhraseSummary) -> Result<(), SessionError> {
        let seg = self.current_segment_mut()?;
        seg.phrases.push(phrase);
        Ok(())
    }

    /// Append a coaching tip to the currently open segment.
    ///
    /// `phrase_index` is interpreted within the current open segment —
    /// it is stored verbatim and not validated, because phrases may be
    /// buffered in the UI before the recorder sees them and out-of-range
    /// values are still meaningful metadata for retrospective analysis.
    pub fn record_tip(
        &mut self,
        phrase_index: usize,
        text: String,
        severity: String,
        category: String,
    ) -> Result<(), SessionError> {
        let seg = self.current_segment_mut()?;
        seg.tips.push(RecordedTip {
            phrase_index,
            text,
            severity,
            category,
            recorded_at: Utc::now(),
        });
        Ok(())
    }

    /// The texts of the most recent `limit` coaching tips recorded this
    /// session, oldest-first within the returned window.
    ///
    /// Threaded into the live coach's [`SessionContext::previous_tips`] so the
    /// next tip can avoid repeating recent advice. Walks every segment in order
    /// and keeps the trailing `limit` texts.
    pub fn recent_tips(&self, limit: usize) -> Vec<String> {
        if limit == 0 {
            return Vec::new();
        }
        let all: Vec<String> = self
            .participants
            .iter()
            .flat_map(|p| p.segments.iter().flat_map(|s| s.tips.iter()))
            .map(|t| t.text.clone())
            .collect();
        let start = all.len().saturating_sub(limit);
        all[start..].to_vec()
    }

    /// Close the currently open segment and open a new one with
    /// `new_instrument` and `new_mode`. Returns the new segment's id.
    ///
    /// Errors with [`SessionError::NoOpenSegment`] if the previous
    /// segment was already closed (shouldn't happen in the MVP flow).
    pub fn switch_instrument(
        &mut self,
        new_instrument: String,
        new_mode: PracticeMode,
    ) -> Result<SegmentId, SessionError> {
        let now = Utc::now();
        {
            // Close the current segment.
            let current = self.current_segment_mut()?;
            current.ended_at = Some(now);
        }
        let new_segment = InstrumentSegment {
            id: SegmentId::new(),
            instrument: new_instrument,
            started_at: now,
            ended_at: None,
            practice_mode: new_mode,
            phrases: Vec::new(),
            tips: Vec::new(),
        };
        let new_id = new_segment.id;
        self.sole_participant_mut()?.segments.push(new_segment);
        Ok(new_id)
    }

    /// Finalise the session and return a [`CompletedSession`] with
    /// authoritative timestamps. Consumes `self` — a recorder is
    /// single-use.
    ///
    /// Any currently-open segment is closed with `ended_at = now`.
    ///
    /// Returns [`SessionError::Empty`] if no phrases were recorded
    /// across any segment.
    pub fn complete(mut self) -> Result<CompletedSession, SessionError> {
        let ended_at = Utc::now();

        // Close any trailing open segment on every participant.
        for p in &mut self.participants {
            if let Some(last) = p.segments.last_mut() {
                if last.ended_at.is_none() {
                    last.ended_at = Some(ended_at);
                }
            }
        }

        let total_phrases: usize = self
            .participants
            .iter()
            .flat_map(|p| p.segments.iter())
            .map(|s| s.phrases.len())
            .sum();
        if total_phrases == 0 {
            return Err(SessionError::Empty);
        }

        // Ensure the structure matches what primary_instrument() expects:
        // at least one participant with at least one segment.
        if self.participants.is_empty() || self.participants.iter().all(|p| p.segments.is_empty()) {
            return Err(SessionError::Empty);
        }

        let duration_secs = (ended_at - self.started_at).num_milliseconds() as f64 / 1000.0;
        Ok(CompletedSession {
            id: self.session_id,
            started_at: self.started_at,
            ended_at,
            duration_secs,
            participants: self.participants,
            score_title: self.score_title,
        })
    }

    /// The session's unique identifier. Stable across the recorder's
    /// lifetime.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Total number of phrases recorded so far, across all segments.
    pub fn phrase_count(&self) -> usize {
        self.participants
            .iter()
            .flat_map(|p| p.segments.iter())
            .map(|s| s.phrases.len())
            .sum()
    }

    /// Total number of tips recorded so far, across all segments.
    pub fn tip_count(&self) -> usize {
        self.participants
            .iter()
            .flat_map(|p| p.segments.iter())
            .map(|s| s.tips.len())
            .sum()
    }

    /// When the session started.
    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    /// Instrument of the currently-open segment.
    pub fn current_instrument(&self) -> Option<&str> {
        self.participants
            .first()
            .and_then(|p| p.segments.last())
            .filter(|s| s.ended_at.is_none())
            .map(|s| s.instrument.as_str())
    }

    /// Title of the score being practised, if this is a score-backed
    /// session. Set once at session start; `None` in free play. Lets the
    /// live coach name the piece in its tips.
    pub fn score_title(&self) -> Option<&str> {
        self.score_title.as_deref()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phrase::{DynamicsStats, PhraseSummary, PitchStats};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // -----------------------------------------------------------------------
    // Mock recap generator
    // -----------------------------------------------------------------------

    struct RecordingMockGenerator {
        last_input: Arc<Mutex<Option<RecapInput>>>,
        call_count: Arc<AtomicUsize>,
        canned: SessionRecap,
    }

    impl RecordingMockGenerator {
        fn new(canned: SessionRecap) -> Self {
            Self {
                last_input: Arc::new(Mutex::new(None)),
                call_count: Arc::new(AtomicUsize::new(0)),
                canned,
            }
        }

        fn last_input_handle(&self) -> Arc<Mutex<Option<RecapInput>>> {
            Arc::clone(&self.last_input)
        }

        fn call_count_handle(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.call_count)
        }
    }

    #[async_trait]
    impl RecapGenerator for RecordingMockGenerator {
        async fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            *self.last_input.lock().unwrap() = Some(input.clone());
            Ok(self.canned.clone())
        }
    }

    struct FailingGenerator;

    #[async_trait]
    impl RecapGenerator for FailingGenerator {
        async fn generate_recap(&self, _input: &RecapInput) -> Result<SessionRecap, SessionError> {
            Err(SessionError::RecapFailed("llm blew up".to_owned()))
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn canned_recap() -> SessionRecap {
        SessionRecap {
            overall_assessment: "Solid warm-up with room to grow.".to_owned(),
            strengths: vec![
                "Consistent tone on middle-register long tones.".to_owned(),
                "Good breath support throughout.".to_owned(),
            ],
            areas_to_improve: vec!["Intonation drifts sharp in the upper register.".to_owned()],
            next_session_suggestions: vec!["Play the C major scale slowly with a drone.".to_owned()],
            duration_secs: 0.0,
            phrase_count: 0,
            instrument: "trumpet".to_owned(),
            fingerprint: None,
            idiom_notes: Vec::new(),
            connections: Vec::new(),
        }
    }

    fn phrase(idx: usize) -> PhraseSummary {
        PhraseSummary {
            phrase_index: idx,
            start_time: idx as f64,
            end_time: idx as f64 + 1.0,
            duration_secs: 1.0,
            note_count: 8,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 435.0,
                max_hz: 445.0,
                range_cents: 40.0,
                pitches: vec![440.0; 8],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.6,
                min_amplitude: 0.4,
                max_amplitude: 0.8,
                dynamic_range: 0.4,
            },
            stability: 0.9,
            score_position: None,
            tone: None,
            key: None,
            onsets_secs: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // New-data-model behaviour tests
    // -----------------------------------------------------------------------

    #[test]
    fn recorder_opens_first_segment_on_new() {
        // new() opens exactly one participant with exactly one open segment.
        let recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        assert_eq!(recorder.participants.len(), 1);
        let p = &recorder.participants[0];
        assert_eq!(p.display_name, "You");
        assert_eq!(p.input_source, InputSource::DefaultMic);
        assert_eq!(p.segments.len(), 1);
        let seg = &p.segments[0];
        assert_eq!(seg.instrument, "trumpet");
        assert!(seg.ended_at.is_none(), "first segment must be open");
        assert_eq!(seg.phrases.len(), 0);
        assert_eq!(seg.tips.len(), 0);
        assert_eq!(recorder.current_instrument(), Some("trumpet"));
    }

    #[test]
    fn switch_instrument_closes_current_and_opens_new() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        let first_id = recorder.participants[0].segments[0].id;
        let new_id = recorder
            .switch_instrument("piano".to_owned(), PracticeMode::default())
            .unwrap();

        let p = &recorder.participants[0];
        assert_eq!(p.segments.len(), 2);
        assert!(
            p.segments[0].ended_at.is_some(),
            "previous segment must be closed after switch"
        );
        assert_eq!(p.segments[0].id, first_id);
        assert_eq!(p.segments[1].id, new_id);
        assert_eq!(p.segments[1].instrument, "piano");
        assert!(p.segments[1].ended_at.is_none(), "new segment must be open");
        assert_ne!(first_id, new_id, "segment ids must be unique");
    }

    #[test]
    fn switch_instrument_preserves_phrase_ordering() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        recorder.record_phrase(phrase(1)).unwrap();
        recorder
            .switch_instrument("piano".to_owned(), PracticeMode::default())
            .unwrap();
        recorder.record_phrase(phrase(2)).unwrap();
        recorder.record_phrase(phrase(3)).unwrap();

        let completed = recorder.complete().unwrap();
        assert_eq!(completed.participants.len(), 1);
        let segs = &completed.participants[0].segments;
        assert_eq!(segs.len(), 2);

        // Phrases 0 & 1 in first (trumpet) segment; 2 & 3 in second (piano).
        assert_eq!(segs[0].instrument, "trumpet");
        assert_eq!(segs[0].phrases.len(), 2);
        assert_eq!(segs[0].phrases[0].phrase_index, 0);
        assert_eq!(segs[0].phrases[1].phrase_index, 1);

        assert_eq!(segs[1].instrument, "piano");
        assert_eq!(segs[1].phrases.len(), 2);
        assert_eq!(segs[1].phrases[0].phrase_index, 2);
        assert_eq!(segs[1].phrases[1].phrase_index, 3);

        // Flattened view honours segment order.
        let flat = completed.all_phrases();
        assert_eq!(flat.len(), 4);
        assert_eq!(
            flat.iter().map(|p| p.phrase_index).collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
        );
    }

    #[test]
    fn record_tip_attaches_to_current_segment_not_sibling() {
        // After a switch, tips must land in the new segment, not the old
        // one. A naive implementation that kept a flat vec would misplace
        // them.
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        recorder
            .record_tip(
                0,
                "old tip".to_owned(),
                "suggestion".to_owned(),
                "tone".to_owned(),
            )
            .unwrap();
        recorder
            .switch_instrument("piano".to_owned(), PracticeMode::default())
            .unwrap();
        recorder.record_phrase(phrase(0)).unwrap();
        recorder
            .record_tip(
                0,
                "new tip".to_owned(),
                "encouragement".to_owned(),
                "dynamics".to_owned(),
            )
            .unwrap();

        let completed = recorder.complete().unwrap();
        let segs = &completed.participants[0].segments;
        assert_eq!(segs[0].tips.len(), 1);
        assert_eq!(segs[0].tips[0].text, "old tip");
        assert_eq!(segs[1].tips.len(), 1);
        assert_eq!(segs[1].tips[0].text, "new tip");
    }

    #[test]
    fn recent_tips_returns_trailing_window_oldest_first_across_segments() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        for i in 0..4 {
            recorder
                .record_tip(
                    i,
                    format!("tip {i}"),
                    "suggestion".to_owned(),
                    "tone".to_owned(),
                )
                .unwrap();
        }

        // No tips requested → empty.
        assert!(recorder.recent_tips(0).is_empty());

        // Fewer recorded than the window → all, oldest-first.
        assert_eq!(
            recorder.recent_tips(10),
            vec!["tip 0", "tip 1", "tip 2", "tip 3"]
        );

        // Capped to the trailing window.
        assert_eq!(recorder.recent_tips(2), vec!["tip 2", "tip 3"]);
    }

    #[test]
    fn complete_closes_open_segment() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();

        let completed = recorder.complete().unwrap();
        let seg = &completed.participants[0].segments[0];
        assert!(
            seg.ended_at.is_some(),
            "final segment must be closed after complete()"
        );
        assert!(
            seg.ended_at.unwrap() >= seg.started_at,
            "ended_at must not be earlier than started_at"
        );
    }

    #[test]
    fn empty_session_still_errors() {
        // The "no phrases ever recorded" check must still fire in the
        // new model — a session with an open segment but no phrases is
        // still empty.
        let recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        let err = recorder.complete().unwrap_err();
        assert!(
            matches!(err, SessionError::Empty),
            "zero-phrase session must return Empty, got {err:?}"
        );
    }

    #[test]
    fn empty_session_after_switch_still_errors() {
        // Regression guard: switching without ever recording a phrase
        // must not accidentally pass the Empty check.
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder
            .switch_instrument("piano".to_owned(), PracticeMode::default())
            .unwrap();
        let err = recorder.complete().unwrap_err();
        assert!(matches!(err, SessionError::Empty));
    }

    #[test]
    fn participant_id_and_segment_id_serde_roundtrip() {
        // Both new id types must round-trip through JSON identically
        // to the existing SessionId — the store relies on this.
        let pid = ParticipantId::new();
        let pjson = serde_json::to_string(&pid).unwrap();
        let pback: ParticipantId = serde_json::from_str(&pjson).unwrap();
        assert_eq!(pid, pback);
        assert_eq!(pid.to_string(), pid.as_str());
        let parsed: ParticipantId = pid.as_str().parse().unwrap();
        assert_eq!(parsed, pid);

        let sid = SegmentId::new();
        let sjson = serde_json::to_string(&sid).unwrap();
        let sback: SegmentId = serde_json::from_str(&sjson).unwrap();
        assert_eq!(sid, sback);
        assert_eq!(sid.to_string(), sid.as_str());
        let parsed: SegmentId = sid.as_str().parse().unwrap();
        assert_eq!(parsed, sid);
    }

    #[test]
    fn input_source_serde_roundtrip() {
        // InputSource uses #[serde(tag = "kind")] to stay forward-compatible
        // with future variants. Verify the wire format is stable.
        let src = InputSource::DefaultMic;
        let json = serde_json::to_string(&src).unwrap();
        assert!(
            json.contains("\"kind\""),
            "InputSource must use tagged enum form, got {json}"
        );
        let back: InputSource = serde_json::from_str(&json).unwrap();
        assert_eq!(src, back);
    }

    // -----------------------------------------------------------------------
    // Existing-behaviour regression tests
    // -----------------------------------------------------------------------

    #[test]
    fn record_phrase_accumulates() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        recorder.record_phrase(phrase(1)).unwrap();
        recorder.record_phrase(phrase(2)).unwrap();
        assert_eq!(recorder.phrase_count(), 3);
    }

    #[tokio::test]
    async fn record_tip_lands_in_input_after_complete() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        let mock = RecordingMockGenerator::new(canned_recap());
        let input_handle = mock.last_input_handle();

        recorder.record_phrase(phrase(0)).unwrap();
        recorder.record_phrase(phrase(1)).unwrap();
        recorder
            .record_tip(
                1,
                "Breathe before this entrance.".to_owned(),
                "suggestion".to_owned(),
                "expression".to_owned(),
            )
            .unwrap();
        assert_eq!(recorder.tip_count(), 1);

        let completed = recorder.complete().unwrap();
        completed.generate_recap(&mock).await.unwrap();
        let captured = input_handle.lock().unwrap().clone().unwrap();
        assert_eq!(captured.tips.len(), 1);
        assert_eq!(captured.tips[0].phrase_index, 1);
        assert_eq!(captured.tips[0].text, "Breathe before this entrance.");
    }

    #[tokio::test]
    async fn generate_recap_flattens_multi_segment_into_input() {
        let canned = canned_recap();
        let mock = RecordingMockGenerator::new(canned);
        let input_handle = mock.last_input_handle();
        let call_count = mock.call_count_handle();

        let mut recorder = SessionRecorder::new("violin".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        recorder
            .record_tip(
                0,
                "Nice bow control.".to_owned(),
                "encouragement".to_owned(),
                "technique".to_owned(),
            )
            .unwrap();
        recorder
            .switch_instrument("piano".to_owned(), PracticeMode::default())
            .unwrap();
        recorder.record_phrase(phrase(1)).unwrap();
        recorder
            .record_tip(
                0,
                "Watch voicing of the left hand.".to_owned(),
                "focus".to_owned(),
                "balance".to_owned(),
            )
            .unwrap();

        let completed = recorder.complete().unwrap();
        completed.generate_recap(&mock).await.unwrap();

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        let captured = input_handle.lock().unwrap().clone().unwrap();
        // Primary instrument — first segment's.
        assert_eq!(captured.instrument, "violin");
        // Phrases & tips flattened across both segments.
        assert_eq!(captured.phrases.len(), 2);
        assert_eq!(captured.tips.len(), 2);
        assert_eq!(captured.tips[0].category, "technique");
        assert_eq!(captured.tips[1].category, "balance");
    }

    #[test]
    fn complete_returns_completed_session_with_authoritative_timestamps() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        let expected_id = recorder.session_id();
        let expected_started = recorder.started_at();
        recorder.record_phrase(phrase(0)).unwrap();

        let completed = recorder.complete().unwrap();
        assert_eq!(completed.id, expected_id);
        assert_eq!(completed.started_at, expected_started);
        assert!(completed.ended_at >= completed.started_at);
        assert_eq!(completed.primary_instrument(), "trumpet");
        assert_eq!(completed.phrase_count(), 1);
    }

    #[test]
    fn complete_calculates_duration_from_start_to_now() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));

        let completed = recorder.complete().unwrap();
        assert!(completed.duration_secs > 0.0);
        assert!(completed.duration_secs < 60.0);
    }

    #[test]
    fn session_id_is_unique_across_recorders() {
        let a = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        let b = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        let c = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        let ids = [a.session_id(), b.session_id(), c.session_id()];
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
        assert_ne!(ids[1], ids[2]);
    }

    #[tokio::test]
    async fn generate_recap_with_profile_threads_profile_into_input() {
        use crate::store::{ExperienceLevel, TasteProfile};

        let mock = RecordingMockGenerator::new(canned_recap());
        let input_handle = mock.last_input_handle();

        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        let completed = recorder.complete().unwrap();

        let profile = TasteProfile {
            genres: vec!["hip-hop".to_owned()],
            artists: vec!["Kendrick Lamar".to_owned()],
            goals: vec!["play in church band".to_owned()],
            experience: ExperienceLevel::Intermediate,
            is_under_13: false,
        };
        completed
            .generate_recap_with_profile(&mock, Some(profile.clone()))
            .await
            .unwrap();

        // The recorder stays profile-agnostic; the join happens here, so the
        // generator must see the profile on the RecapInput.
        let captured = input_handle.lock().unwrap().clone().unwrap();
        assert_eq!(captured.taste_profile, Some(profile));
    }

    #[tokio::test]
    async fn generate_recap_with_no_profile_matches_plain_generate_recap() {
        // Cold start: passing None must behave exactly like generate_recap —
        // no profile on the input, equivalent to the existing behavior.
        let mock = RecordingMockGenerator::new(canned_recap());
        let input_handle = mock.last_input_handle();

        let mut recorder = SessionRecorder::new("voice".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        let completed = recorder.complete().unwrap();

        completed
            .generate_recap_with_profile(&mock, None)
            .await
            .unwrap();

        let captured = input_handle.lock().unwrap().clone().unwrap();
        assert!(
            captured.taste_profile.is_none(),
            "cold start must leave the recap input profile-free"
        );
    }

    #[tokio::test]
    async fn recap_generator_failure_propagates_as_session_error() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        let completed = recorder.complete().unwrap();

        let err = completed
            .generate_recap(&FailingGenerator)
            .await
            .unwrap_err();
        match err {
            SessionError::RecapFailed(msg) => assert_eq!(msg, "llm blew up"),
            other => panic!("expected RecapFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn generate_recap_overwrites_authoritative_fields() {
        let mut lying_recap = canned_recap();
        lying_recap.duration_secs = 999.0;
        lying_recap.phrase_count = 42;
        lying_recap.instrument = "tuba".to_owned();

        let mock = RecordingMockGenerator::new(lying_recap);
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        recorder.record_phrase(phrase(1)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));

        let completed = recorder.complete().unwrap();
        let recap = completed.generate_recap(&mock).await.unwrap();

        assert_eq!(recap.instrument, "trumpet");
        assert_eq!(recap.phrase_count, 2);
        assert!((recap.duration_secs - completed.duration_secs).abs() < 1e-9);
        assert!(!recap.overall_assessment.is_empty());
    }

    #[tokio::test]
    async fn session_data_survives_recap_generator_failure() {
        let mut recorder = SessionRecorder::new("flute".to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase(0)).unwrap();
        recorder.record_phrase(phrase(1)).unwrap();

        let completed = recorder.complete().unwrap();
        let saved_id = completed.id;
        let saved_count = completed.phrase_count();

        let err = completed
            .generate_recap(&FailingGenerator)
            .await
            .unwrap_err();
        assert!(matches!(err, SessionError::RecapFailed(_)));

        assert_eq!(completed.id, saved_id);
        assert_eq!(completed.phrase_count(), saved_count);

        let retry_mock = RecordingMockGenerator::new(canned_recap());
        let recap = completed.generate_recap(&retry_mock).await.unwrap();
        assert_eq!(recap.phrase_count, saved_count);
    }

    // -----------------------------------------------------------------------
    // Id type regression tests
    // -----------------------------------------------------------------------

    #[test]
    fn session_id_roundtrip_via_from_str() {
        let id = SessionId::new();
        let parsed: SessionId = id.as_str().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn session_id_display_matches_as_str() {
        let id = SessionId::new();
        assert_eq!(format!("{id}"), id.as_str());
    }

    #[test]
    fn session_id_serde_roundtrip() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn recap_serde_preserves_vectors() {
        let recap = canned_recap();
        let json = serde_json::to_string(&recap).unwrap();
        let parsed: SessionRecap = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, recap);
    }
}
