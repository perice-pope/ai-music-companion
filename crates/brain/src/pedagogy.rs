//! # Pedagogy corpus — method-book technique guidance (#454 S1)
//!
//! A structured library of instrument-family technique guidance drawn from the
//! canonical method books (Arban, Schlossberg, Suzuki, Hanon, …), shipped as
//! data files in the repo-root `pedagogy/` directory — one JSON file per
//! family, the same "adding content = adding a data file" convention as
//! `profiles/`.
//!
//! ## Shipping mechanism (a documented decision)
//!
//! The corpus is embedded at **compile time** via `include_str!`, the same
//! pattern the `idiom` crate uses for its seed corpus
//! (`crates/idiom/src/corpus.rs`). `profiles/` ships as runtime files because
//! the packaged app enumerates the user's instrument catalog from disk and
//! needed resource-dir resolution (#112); the pedagogy corpus has no
//! runtime-file requirement, so compile-time embedding is the simplest robust
//! choice — no bundler resource config, no missing-file failure mode, and
//! cargo rebuilds when a JSON changes.
//!
//! ## The copyright gate (the house rule from #454)
//!
//! Every source is tagged with a [`SourceStatus`]:
//!
//! - **`pd`** (public domain — Arban 1864, Hanon 1873, …): entries may carry
//!   verbatim quotes, including an explicit `quote` field.
//! - **`paraphrase-only`** (in copyright — Schlossberg 1937, Suzuki, …):
//!   attributed paraphrase of technique *facts* only, plus a "see [book],
//!   section N" pointer to the player's own copy. [`validate_entries`]
//!   **rejects** a `quote` field and any quoted run of more than
//!   [`MAX_QUOTED_WORDS`] consecutive words in the guidance text.
//!
//! The gate runs in `cargo test` (this module's tests validate the real
//! embedded corpus), so CI refuses verbatim text in `paraphrase-only` entries.
//! It is a tripwire, not a lawyer: human review of paraphrase wording remains
//! part of corpus-PR review — the gate exists so verbatim paste can't land
//! silently.
//!
//! ## Selection (S2)
//!
//! [`select_pedagogy`] is the evidence-gated selection engine: it derives
//! corpus trigger tags from what a session's [`MusicalFingerprint`]
//! **actually measured** (see [`evidence_tags`] — every mapping names its
//! measured field and threshold; tags without a measurement are never
//! derived), filters the corpus to the player's family, and picks the entry
//! with the most trigger overlap, ties broken by smallest id. Below the
//! evidence bars it returns `None` — silence > lies. Surfacing (S3, behind
//! #453's coaching box) builds on this.

use crate::fingerprint::MusicalFingerprint;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

/// Instrument family, matching the display-name casing the desktop shell's
/// `instrument_family_for` already routes over IPC ("Piano" → "Keyboard").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Family {
    Brass,
    Strings,
    Voice,
    Woodwind,
    Keyboard,
}

impl Family {
    /// All families the corpus must cover.
    pub const ALL: [Family; 5] = [
        Family::Brass,
        Family::Strings,
        Family::Voice,
        Family::Woodwind,
        Family::Keyboard,
    ];

    /// Exact display-name parse ("Brass" … "Keyboard") — the casing the
    /// desktop shell's `instrument_family_for` emits over IPC (it returns
    /// exactly these strings, or "" for an unknown instrument). Anything
    /// else — empty, lowercase, "Percussion" (no corpus file) — is `None`,
    /// and selection stays calmly silent.
    pub fn from_display_name(name: &str) -> Option<Family> {
        match name {
            "Brass" => Some(Family::Brass),
            "Strings" => Some(Family::Strings),
            "Voice" => Some(Family::Voice),
            "Woodwind" => Some(Family::Woodwind),
            "Keyboard" => Some(Family::Keyboard),
            _ => None,
        }
    }
}

/// Copyright status of a source book — the gate's pivot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceStatus {
    /// Public domain: quotes, exercise text, and notation may ship verbatim.
    Pd,
    /// In copyright: attributed paraphrase of technique facts only.
    ParaphraseOnly,
}

/// The method book a piece of guidance is drawn from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub title: String,
    pub author: String,
    pub year: u16,
    pub status: SourceStatus,
    /// Which part of the book ("Interval studies", "Part 1, Exercises 1-20").
    pub section: String,
}

/// One corpus entry: a technique topic, its source, the guidance text, and
/// the evidence tags (the S2 selection seam) that make it relevant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PedagogyEntry {
    /// Stable unique id, kebab-case (`brass-arban-interval-bottom-note`).
    pub id: String,
    pub family: Family,
    /// Human-facing technique topic ("Ascending interval leaps").
    pub topic: String,
    pub source: SourceRef,
    /// The guidance itself — a PD quote/close paraphrase, or for
    /// `paraphrase-only` sources an attributed paraphrase in the coach's
    /// voice with a pointer to the player's own copy.
    pub guidance: String,
    /// Optional verbatim quote — permitted for `pd` sources only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// Optional editorial note — edition/translation caveats and similar
    /// (e.g. "the PD status covers the French original; English translations
    /// are largely still in copyright"). Never surfaced to the player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Evidence tags (kebab-case) a session fingerprint can match against,
    /// e.g. `ascending-leap-attacks`, `uneven-eighths`, `pitch-sag-sustain`.
    pub triggers: Vec<String>,
}

/// Longest quoted run permitted in `paraphrase-only` guidance, in words.
/// Anything longer trips [`PedagogyError::VerbatimInParaphraseOnly`].
pub const MAX_QUOTED_WORDS: usize = 15;

/// Corpus loading / validation errors.
#[derive(Debug, Error)]
pub enum PedagogyError {
    #[error("failed to parse {file}: {message}")]
    Parse { file: String, message: String },
    #[error("entry in {file} has family {found:?}, but that file is for {expected:?}")]
    FamilyFileMismatch {
        file: String,
        expected: Family,
        found: Family,
    },
    #[error("duplicate entry id: {0}")]
    DuplicateId(String),
    #[error("entry {entry}: field `{field}` is empty")]
    EmptyField { entry: String, field: &'static str },
    #[error("entry {entry}: triggers must be a nonempty list of nonempty tags")]
    EmptyTrigger { entry: String },
    #[error(
        "entry {entry}: `quote` field on a paraphrase-only source — in-copyright \
         books get attributed paraphrase only (#454 house rule)"
    )]
    QuoteFieldInParaphraseOnly { entry: String },
    #[error(
        "entry {entry}: guidance contains a quoted run of {words} words \
         (max {MAX_QUOTED_WORDS}) — looks like verbatim text from an \
         in-copyright source (#454 house rule)"
    )]
    VerbatimInParaphraseOnly { entry: String, words: usize },
}

/// The embedded corpus: one file per family, from the repo-root `pedagogy/`
/// directory. Adding content = editing a data file; adding a family = one new
/// row here.
const FAMILY_FILES: [(&str, Family, &str); 5] = [
    (
        "pedagogy/brass.json",
        Family::Brass,
        include_str!("../../../pedagogy/brass.json"),
    ),
    (
        "pedagogy/strings.json",
        Family::Strings,
        include_str!("../../../pedagogy/strings.json"),
    ),
    (
        "pedagogy/voice.json",
        Family::Voice,
        include_str!("../../../pedagogy/voice.json"),
    ),
    (
        "pedagogy/woodwind.json",
        Family::Woodwind,
        include_str!("../../../pedagogy/woodwind.json"),
    ),
    (
        "pedagogy/keyboard.json",
        Family::Keyboard,
        include_str!("../../../pedagogy/keyboard.json"),
    ),
];

/// Load the embedded corpus.
///
/// Infallible by construction: the corpus is embedded at compile time and the
/// CI gate (`cargo test` in this module) validates the exact same bytes, so a
/// failure here is unreachable in any build that passed CI.
pub fn load_corpus() -> Vec<PedagogyEntry> {
    try_load_corpus().expect("embedded pedagogy corpus is validated by the CI gate")
}

/// Load and validate the embedded corpus, surfacing any schema or
/// copyright-gate violation.
pub fn try_load_corpus() -> Result<Vec<PedagogyEntry>, PedagogyError> {
    let mut entries = Vec::new();
    for (file, expected_family, json) in FAMILY_FILES {
        let parsed: Vec<PedagogyEntry> =
            serde_json::from_str(json).map_err(|e| PedagogyError::Parse {
                file: file.to_owned(),
                message: e.to_string(),
            })?;
        for entry in &parsed {
            if entry.family != expected_family {
                return Err(PedagogyError::FamilyFileMismatch {
                    file: file.to_owned(),
                    expected: expected_family,
                    found: entry.family,
                });
            }
        }
        entries.extend(parsed);
    }
    validate_entries(&entries)?;
    Ok(entries)
}

/// Validate schema invariants and the #454 copyright gate over a set of
/// entries. This is the function the CI gate tests call — both on the real
/// embedded corpus and on planted-violation fixtures.
pub fn validate_entries(entries: &[PedagogyEntry]) -> Result<(), PedagogyError> {
    let mut seen_ids = HashSet::new();
    for entry in entries {
        let nonempty = [
            (entry.id.as_str(), "id"),
            (entry.topic.as_str(), "topic"),
            (entry.guidance.as_str(), "guidance"),
            (entry.source.title.as_str(), "source.title"),
            (entry.source.author.as_str(), "source.author"),
            (entry.source.section.as_str(), "source.section"),
        ];
        for (value, field) in nonempty {
            if value.trim().is_empty() {
                return Err(PedagogyError::EmptyField {
                    entry: display_id(entry),
                    field,
                });
            }
        }
        if !seen_ids.insert(entry.id.clone()) {
            return Err(PedagogyError::DuplicateId(entry.id.clone()));
        }
        if entry.triggers.is_empty() || entry.triggers.iter().any(|t| t.trim().is_empty()) {
            return Err(PedagogyError::EmptyTrigger {
                entry: entry.id.clone(),
            });
        }
        if entry.source.status == SourceStatus::ParaphraseOnly {
            if entry.quote.is_some() {
                return Err(PedagogyError::QuoteFieldInParaphraseOnly {
                    entry: entry.id.clone(),
                });
            }
            let words = longest_quoted_run_words(&entry.guidance);
            if words > MAX_QUOTED_WORDS {
                return Err(PedagogyError::VerbatimInParaphraseOnly {
                    entry: entry.id.clone(),
                    words,
                });
            }
        }
    }
    Ok(())
}

/// An entry's id for error messages, or a placeholder when the id itself is
/// the empty field being reported.
fn display_id(entry: &PedagogyEntry) -> String {
    if entry.id.trim().is_empty() {
        "<missing id>".to_owned()
    } else {
        entry.id.clone()
    }
}

/// The longest run of words enclosed in double quotes (ASCII `"` or curly
/// `\u{201C}`/`\u{201D}`) in `text`, in whitespace-separated words.
///
/// Conservative on unbalanced quotes: text after an unclosed opening quote
/// counts as quoted through end-of-string — the heuristic over-catches, never
/// under-catches.
fn longest_quoted_run_words(text: &str) -> usize {
    let mut inside = false;
    let mut current = String::new();
    let mut max_words = 0;
    for ch in text.chars() {
        if matches!(ch, '"' | '\u{201C}' | '\u{201D}') {
            if inside {
                max_words = max_words.max(current.split_whitespace().count());
                current.clear();
            }
            inside = !inside;
        } else if inside {
            current.push(ch);
        }
    }
    if inside {
        max_words = max_words.max(current.split_whitespace().count());
    }
    max_words
}

// ───────────────────────────────────────────────────────────────────────────
// S2: evidence-gated selection (#454)
//
// The mapping below is an AUDIT of `MusicalFingerprint`, not a wishlist:
// only 7 of the corpus's 34 trigger tags have a measured evidence source
// today, and only those 7 can ever be derived. Everything else stays silent
// until ears/brain measure it (the full not-mapped audit lives in
// docs/specs/454-s2-selection.md §4.1). Silence > lies.
// ───────────────────────────────────────────────────────────────────────────

/// E1–E3 count bar: mirrors `coaching::aggregate_intonation`'s `MIN_NOTES`
/// gate (12). The fingerprint's own docs say building code must reuse those
/// gates, never loosen them; re-checking here also guards fingerprints that
/// arrive from persistence rather than the live gates.
pub const MIN_INTONATION_NOTES: u32 = 12;

/// E4–E5 count bar: mirrors `coaching::aggregate_groove`'s `MIN_ONSETS`
/// gate (6), for the same reasons as [`MIN_INTONATION_NOTES`].
pub const MIN_GROOVE_ONSETS: u32 = 6;

/// E1 threshold: the session's **signed** mean pitch deviation is at least
/// one full in-tune tolerance ([`theory::DEFAULT_IN_TUNE_TOLERANCE_CENTS`],
/// 15 ¢) *flat*. The sign is measured, so "sagging below center" is a fact,
/// not a guess.
pub const FLAT_SUSTAIN_MEAN_CENTS: f32 = -theory::DEFAULT_IN_TUNE_TOLERANCE_CENTS;

/// E2 threshold: fewer than half the observed notes landed within the
/// in-tune tolerance — the majority of the session's pitch was off center.
pub const DRIFT_MAX_IN_TUNE_RATIO: f32 = 0.5;

/// E3 threshold: mean *absolute* pitch error of a quarter semitone or worse —
/// at that magnitude the intervals between successive notes cannot be
/// landing accurately.
pub const INACCURATE_MEAN_ABS_CENTS: f32 = 25.0;

/// E4 threshold: below 0.7 is exactly the band `coaching::describe_groove`
/// already reads to the player as "uneven" — same dial, same meaning.
pub const UNEVEN_TIMING_CONSISTENCY: f32 = 0.7;

/// E5 threshold: an inter-onset-interval coefficient of variation above 0.5
/// (`timing_consistency` = 1 − CV) — not just uneven subdivisions, the pulse
/// itself is unstable.
pub const UNSTABLE_TIMING_CONSISTENCY: f32 = 0.5;

/// E6 threshold: the session-mean noise component (`ToneDescriptor::air_noise`,
/// 0..1) dominates the sound.
pub const NOISY_TONE_AIR_NOISE: f32 = 0.6;

/// E7 threshold: the session-mean tonal-core focus
/// (`ToneDescriptor::core_clarity`, 0..1) failed to hold.
pub const UNFOCUSED_CORE_CLARITY: f32 = 0.35;

/// Tags derived from the intonation dimension (E1–E3). Dropped before
/// matching for fixed-pitch families ([`crate::coaching::fixed_pitch_family`]):
/// on those instruments measured cents are the INSTRUMENT's tuning, never the
/// player's technique (#417-4/#389), so pitch pedagogy would be a lie.
const INTONATION_DEFICIT_TAGS: [&str; 3] =
    ["pitch-sag-sustain", "pitch-drift", "interval-accuracy"];

/// Derive corpus evidence tags from what `fingerprint` **measured**.
///
/// Each mapping fires only when its dimension is present, its count bar is
/// met, and its threshold is crossed — the rationale for every row (measured
/// field + threshold) is on the constants above and in the spec's mapping
/// table. Returns tags in a fixed E1..E7 order (deterministic), empty when
/// nothing crossed a bar. Tags the fingerprint carries no evidence for
/// (attack quality, rushing direction, …) are never derived.
pub fn evidence_tags(fingerprint: &MusicalFingerprint) -> Vec<&'static str> {
    let mut tags = Vec::new();

    if let Some(intonation) = &fingerprint.intonation {
        if intonation.note_count >= MIN_INTONATION_NOTES {
            // E1: measured signed mean (`mean_cents`) ≤ −15 ¢ → the pitch is
            // sagging flat across the session's sustained playing.
            if intonation.mean_cents <= FLAT_SUSTAIN_MEAN_CENTS {
                tags.push("pitch-sag-sustain");
            }
            // E2: measured `in_tune_ratio` < 0.5 → most notes off center.
            if intonation.in_tune_ratio < DRIFT_MAX_IN_TUNE_RATIO {
                tags.push("pitch-drift");
            }
            // E3: measured `mean_abs_cents` ≥ 25 ¢ → pitch placement (and so
            // the intervals between notes) is inaccurate on average.
            if intonation.mean_abs_cents >= INACCURATE_MEAN_ABS_CENTS {
                tags.push("interval-accuracy");
            }
        }
    }

    if let Some(groove) = &fingerprint.groove {
        if groove.onset_count >= MIN_GROOVE_ONSETS {
            // E4: measured `timing_consistency` < 0.7 — the band the recap
            // already calls "uneven". NOTE: deliberately NOT mapped to
            // "rushing-runs": rushing claims a direction, and 1 − CV is
            // undirected (lay-back/rush needs a beat grid we don't have).
            if groove.timing_consistency < UNEVEN_TIMING_CONSISTENCY {
                tags.push("uneven-eighths");
            }
            // E5: measured `timing_consistency` < 0.5 — the pulse itself is
            // unstable, in addition to (never instead of) E4.
            if groove.timing_consistency < UNSTABLE_TIMING_CONSISTENCY {
                tags.push("tempo-instability");
            }
        }
    }

    if let Some(tone) = &fingerprint.tone {
        // E6: measured session-mean `air_noise` ≥ 0.6 — noise dominates the
        // tone. One measurement, two family readings: voice pedagogy carries
        // `breathy-onset`, strings pedagogy carries `scratchy-tone`; family
        // filtering in [`select_entry`] keeps only the one that applies.
        if tone.air_noise >= NOISY_TONE_AIR_NOISE {
            tags.push("breathy-onset");
            tags.push("scratchy-tone");
        }
        // E7: measured session-mean `core_clarity` ≤ 0.35 — the tonal core
        // never held focus; `tone-inconsistency` is the corpus's fundamental
        // tone-production tag (tonalization / sonorité / mouthpiece work).
        if tone.core_clarity <= UNFOCUSED_CORE_CLARITY {
            tags.push("tone-inconsistency");
        }
    }

    tags
}

/// Deterministic matching core: among `entries` of `family` whose `triggers`
/// intersect `tags`, pick the one with the **most trigger overlap**, ties
/// broken by **lexicographically smallest id**. `None` when nothing matches.
///
/// Exposed separately from [`select_pedagogy`] so tests (and future callers
/// with their own evidence) can exercise the rule with tags the fingerprint
/// cannot yet produce.
pub fn select_entry<'a>(
    entries: &'a [PedagogyEntry],
    family: Family,
    tags: &[&str],
) -> Option<&'a PedagogyEntry> {
    let mut best: Option<(usize, &PedagogyEntry)> = None;
    for entry in entries.iter().filter(|e| e.family == family) {
        let overlap = entry
            .triggers
            .iter()
            .filter(|t| tags.contains(&t.as_str()))
            .count();
        if overlap == 0 {
            continue;
        }
        let better = match best {
            None => true,
            Some((best_overlap, best_entry)) => {
                overlap > best_overlap || (overlap == best_overlap && entry.id < best_entry.id)
            }
        };
        if better {
            best = Some((overlap, entry));
        }
    }
    best.map(|(_, entry)| entry)
}

/// Family-aware evidence seam: [`evidence_tags`] with the fixed-pitch
/// exemption applied. For families where the player cannot bend pitch
/// ([`crate::coaching::fixed_pitch_family`] — reused, never duplicated),
/// the intonation-derived tags (E1–E3) are stripped: measured cents there
/// are the INSTRUMENT's tuning, never the player's technique (#417-4/#389)
/// — the same honesty rule that keeps tuner advice out of keyboard recaps.
///
/// `pub(crate)` so the exemption is pinned by a test that cannot be
/// satisfied by corpus shape (today no Keyboard entry carries an E1–E3
/// trigger, so end-to-end selection alone could not catch this strip
/// being deleted; S4 corpus growth is data-only and must not become this
/// path's only guard).
pub(crate) fn evidence_tags_for_family(
    family: &str,
    fingerprint: &MusicalFingerprint,
) -> Vec<&'static str> {
    let mut tags = evidence_tags(fingerprint);
    if crate::coaching::fixed_pitch_family(family) {
        tags.retain(|tag| !INTONATION_DEFICIT_TAGS.contains(tag));
    }
    tags
}

/// The S2 selection engine: the player's family (display name, as
/// `instrument_family_for` emits it) + the session's measured fingerprint →
/// the single most relevant corpus entry, or `None` below the evidence bars.
///
/// Pure and offline: derive the family-aware evidence tags
/// ([`evidence_tags_for_family`] — the fixed-pitch exemption lives there),
/// then run [`select_entry`] over the embedded corpus.
/// Unknown/empty family, no tags, or no family entry matching the evidence →
/// `None` — no tip without a measured trigger.
pub fn select_pedagogy(family: &str, fingerprint: &MusicalFingerprint) -> Option<PedagogyEntry> {
    let parsed = Family::from_display_name(family)?;
    let tags = evidence_tags_for_family(family, fingerprint);
    if tags.is_empty() {
        return None;
    }
    let corpus = load_corpus();
    select_entry(&corpus, parsed, &tags).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal valid entry to mutate in fixtures.
    fn base_entry(id: &str, status: SourceStatus) -> PedagogyEntry {
        PedagogyEntry {
            id: id.to_owned(),
            family: Family::Brass,
            topic: "Long tones".to_owned(),
            source: SourceRef {
                title: "Daily Drills and Technical Studies".to_owned(),
                author: "Max Schlossberg".to_owned(),
                year: 1937,
                status,
                section: "Long-tone drills".to_owned(),
            },
            guidance: "Keep the pitch level while the volume changes.".to_owned(),
            quote: None,
            note: None,
            triggers: vec!["pitch-sag-sustain".to_owned()],
        }
    }

    fn quoted_words(n: usize) -> String {
        let words = vec!["word"; n].join(" ");
        format!("He wrote \"{words}\" in the drills.")
    }

    // AC1 + AC6: the real embedded corpus loads and is gate-clean.
    #[test]
    fn corpus_loads_and_validates() {
        let corpus = try_load_corpus().expect("embedded corpus must load and pass the gate");
        assert!(!corpus.is_empty());
        // load_corpus is the infallible wrapper over the same bytes.
        assert_eq!(load_corpus(), corpus);
    }

    // AC2: all five families, with the seed minimums from the spec.
    #[test]
    fn family_coverage_meets_minimums() {
        let corpus = load_corpus();
        let count = |f: Family| corpus.iter().filter(|e| e.family == f).count();
        let minimums = [
            (Family::Brass, 6),
            (Family::Strings, 4),
            (Family::Voice, 3),
            (Family::Woodwind, 3),
            (Family::Keyboard, 3),
        ];
        for (family, min) in minimums {
            assert!(
                count(family) >= min,
                "family {family:?} has {} entries, spec minimum is {min}",
                count(family)
            );
        }
        for family in Family::ALL {
            assert!(count(family) > 0, "family {family:?} missing from corpus");
        }
    }

    // AC1: trigger tags are the S2 selection seam — every entry must have
    // at least one, and no tag may be blank.
    #[test]
    fn every_entry_has_nonempty_triggers() {
        for entry in load_corpus() {
            assert!(
                !entry.triggers.is_empty(),
                "entry {} has no triggers — S2 could never select it",
                entry.id
            );
            for tag in &entry.triggers {
                assert!(
                    !tag.trim().is_empty(),
                    "entry {} has a blank trigger tag",
                    entry.id
                );
            }
        }
    }

    // AC3: planted violation — a `quote` field on a paraphrase-only source.
    #[test]
    fn gate_rejects_quote_field_in_paraphrase_only() {
        let mut entry = base_entry("fixture-planted-quote", SourceStatus::ParaphraseOnly);
        entry.quote = Some("Start the tone softly and let it grow.".to_owned());
        let err = validate_entries(&[entry]).unwrap_err();
        assert!(
            matches!(err, PedagogyError::QuoteFieldInParaphraseOnly { ref entry }
                if entry == "fixture-planted-quote"),
            "expected QuoteFieldInParaphraseOnly, got: {err}"
        );
    }

    // MF1 (review round 1): the gate's dial is a licensing decision. Pin it
    // so a "harmless" constant bump cannot pass review as a refactor.
    #[test]
    fn gate_dial_is_pinned_at_15() {
        assert_eq!(
            MAX_QUOTED_WORDS, 15,
            "the copyright gate's core dial — changing it is a licensing decision, not a refactor"
        );
    }

    // MF1 (review round 1): a hardcoded 16-word quoted run must be rejected.
    // Deliberately NOT expressed relative to MAX_QUOTED_WORDS — the
    // constant-relative fixtures below would survive a 15→50 mutation; this
    // one dies with it.
    #[test]
    fn gate_rejects_hardcoded_sixteen_word_quote() {
        let mut entry = base_entry("fixture-hardcoded-sixteen", SourceStatus::ParaphraseOnly);
        entry.guidance = "He wrote \"one two three four five six seven eight nine ten \
                          eleven twelve thirteen fourteen fifteen sixteen\" in the drills."
            .to_owned();
        let err = validate_entries(&[entry]).unwrap_err();
        assert!(
            matches!(
                err,
                PedagogyError::VerbatimInParaphraseOnly { words: 16, .. }
            ),
            "a 16-word verbatim run must trip the gate at any dial setting \
             the house rule permits, got: {err}"
        );
    }

    // AC4: planted violation — a 16-word quoted run in paraphrase-only guidance.
    #[test]
    fn gate_rejects_long_verbatim_in_paraphrase_only() {
        let mut entry = base_entry("fixture-planted-verbatim", SourceStatus::ParaphraseOnly);
        entry.guidance = quoted_words(MAX_QUOTED_WORDS + 1);
        let err = validate_entries(&[entry]).unwrap_err();
        assert!(
            matches!(err, PedagogyError::VerbatimInParaphraseOnly { words, .. }
                if words == MAX_QUOTED_WORDS + 1),
            "expected VerbatimInParaphraseOnly, got: {err}"
        );
    }

    // AC4 boundary: exactly 15 quoted words is still allowed (short quoted
    // phrases like a named syllable or drill title are legitimate).
    #[test]
    fn gate_allows_short_quotes_in_paraphrase_only() {
        let mut entry = base_entry("fixture-short-quote", SourceStatus::ParaphraseOnly);
        entry.guidance = quoted_words(MAX_QUOTED_WORDS);
        validate_entries(&[entry]).expect("15 quoted words is within the gate");
    }

    // AC4: curly quotes are caught the same as ASCII quotes.
    #[test]
    fn gate_catches_curly_quotes() {
        let mut entry = base_entry("fixture-curly-quotes", SourceStatus::ParaphraseOnly);
        let words = vec!["word"; MAX_QUOTED_WORDS + 1].join(" ");
        entry.guidance = format!("He wrote \u{201C}{words}\u{201D} in the drills.");
        let err = validate_entries(&[entry]).unwrap_err();
        assert!(
            matches!(err, PedagogyError::VerbatimInParaphraseOnly { .. }),
            "curly quotes must not slip past the gate, got: {err}"
        );
    }

    // AC5: pd sources may quote freely — quote field and long quoted runs.
    #[test]
    fn pd_entries_may_quote() {
        let mut entry = base_entry("fixture-pd-quotes", SourceStatus::Pd);
        entry.quote = Some(
            "The mouthpiece should be placed in the middle of the lips, two-thirds on \
             the lower lip, and one-third on the upper lip."
                .to_owned(),
        );
        entry.guidance = quoted_words(MAX_QUOTED_WORDS + 20);
        validate_entries(&[entry]).expect("pd entries may carry verbatim quotes");
    }

    // AC7 (strengthened in review round 1): every paraphrase-only entry names
    // its author's SURNAME in the guidance copy — attribution lives in the
    // text the player will read, and a loose any-word match is too easy to
    // satisfy by accident.
    #[test]
    fn paraphrase_entries_carry_attribution_in_copy() {
        for entry in load_corpus() {
            if entry.source.status != SourceStatus::ParaphraseOnly {
                continue;
            }
            // Surname = last word of the author, ignoring any parenthetical
            // ("G. B. Lamperti (transcribed by W. E. Brown)" → "lamperti").
            let surname = entry
                .source
                .author
                .split('(')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .last()
                .unwrap_or("")
                .to_lowercase();
            assert!(
                !surname.is_empty(),
                "entry {} has an author with no surname",
                entry.id
            );
            assert!(
                entry.guidance.to_lowercase().contains(&surname),
                "paraphrase-only entry {} must name the author's surname \
                 ({surname}) in the guidance text",
                entry.id
            );
        }
    }

    // SF3 (review round 1): a paraphrase-only entry that cites the book's
    // exercises/drills sends the player to their own copy — the corpus never
    // substitutes for the in-copyright book.
    #[test]
    fn exercise_citing_paraphrase_entries_point_to_own_copy() {
        for entry in load_corpus() {
            if entry.source.status != SourceStatus::ParaphraseOnly {
                continue;
            }
            let section = entry.source.section.to_lowercase();
            if section.contains("exercise") || section.contains("drill") {
                assert!(
                    entry.guidance.to_lowercase().contains("your own"),
                    "paraphrase-only entry {} cites exercises/drills \
                     ({:?}) but has no your-own-copy pointer in the guidance",
                    entry.id,
                    entry.source.section
                );
            }
        }
    }

    // Edge: duplicate ids across the corpus are rejected.
    #[test]
    fn gate_rejects_duplicate_ids() {
        let a = base_entry("fixture-dup", SourceStatus::Pd);
        let b = base_entry("fixture-dup", SourceStatus::Pd);
        let err = validate_entries(&[a, b]).unwrap_err();
        assert!(matches!(err, PedagogyError::DuplicateId(ref id) if id == "fixture-dup"));
    }

    // Edge: a blank trigger tag is rejected.
    #[test]
    fn gate_rejects_empty_trigger() {
        let mut entry = base_entry("fixture-blank-tag", SourceStatus::Pd);
        entry.triggers = vec!["uneven-eighths".to_owned(), "  ".to_owned()];
        let err = validate_entries(&[entry]).unwrap_err();
        assert!(matches!(err, PedagogyError::EmptyTrigger { ref entry }
            if entry == "fixture-blank-tag"));
    }

    // Edge: an empty triggers list is rejected.
    #[test]
    fn gate_rejects_missing_triggers() {
        let mut entry = base_entry("fixture-no-tags", SourceStatus::Pd);
        entry.triggers = Vec::new();
        let err = validate_entries(&[entry]).unwrap_err();
        assert!(matches!(err, PedagogyError::EmptyTrigger { .. }));
    }

    // Edge: unknown JSON keys are rejected, so a renamed verbatim-carrying
    // field (`excerpt`, `verbatim`) can't sneak content past the gate.
    #[test]
    fn unknown_fields_rejected() {
        let json = r#"{
            "id": "fixture-excerpt-smuggle",
            "family": "Brass",
            "topic": "Long tones",
            "source": {
                "title": "Daily Drills and Technical Studies",
                "author": "Max Schlossberg",
                "year": 1937,
                "status": "paraphrase-only",
                "section": "Long-tone drills"
            },
            "guidance": "Keep the pitch level while the volume changes.",
            "excerpt": "a smuggled verbatim passage",
            "triggers": ["pitch-sag-sustain"]
        }"#;
        let parsed: Result<PedagogyEntry, _> = serde_json::from_str(json);
        assert!(
            parsed.is_err(),
            "unknown fields must be rejected so verbatim text can't be smuggled"
        );
    }

    // Edge: unbalanced quotes are treated conservatively (tail counts as
    // quoted), so the heuristic over-catches rather than under-catches.
    #[test]
    fn unbalanced_quote_counts_to_end_of_string() {
        let mut entry = base_entry("fixture-unbalanced", SourceStatus::ParaphraseOnly);
        let words = vec!["word"; MAX_QUOTED_WORDS + 1].join(" ");
        entry.guidance = format!("He wrote \"{words}");
        let err = validate_entries(&[entry]).unwrap_err();
        assert!(matches!(
            err,
            PedagogyError::VerbatimInParaphraseOnly { .. }
        ));
    }

    // Heuristic detail: runs are counted in whitespace-separated words, and
    // the longest run wins.
    #[test]
    fn word_counting_is_whitespace_word_based() {
        assert_eq!(longest_quoted_run_words("no quotes here"), 0);
        assert_eq!(longest_quoted_run_words("say \"tu\" lightly"), 1);
        assert_eq!(
            longest_quoted_run_words("\"one two\" then \"one two three\""),
            3
        );
    }

    // ─── S2: evidence-gated selection (#454) ───────────────────────────────

    /// An all-dimensions-measured, HEALTHY fingerprint: every value sits on
    /// the healthy side of every S2 threshold. Deficit fixtures mutate this.
    fn healthy_fingerprint() -> MusicalFingerprint {
        MusicalFingerprint {
            tone: Some(tone::ToneDescriptor {
                brightness: 0.5,
                warmth: 0.5,
                air_noise: 0.2,
                core_clarity: 0.8,
                vibrato_quality: 0.5,
            }),
            key: None,
            key_claim: None,
            intonation: Some(theory::IntonationSummary {
                note_count: 30,
                mean_cents: 2.0,
                mean_abs_cents: 8.0,
                in_tune_ratio: 0.9,
                tendencies: Vec::new(),
            }),
            groove: Some(groove::GrooveDescriptor {
                tempo_bpm: Some(100.0),
                swing_ratio: Some(1.0),
                mean_ioi_secs: 0.3,
                timing_consistency: 0.9,
                onset_count: 24,
            }),
        }
    }

    fn empty_fingerprint() -> MusicalFingerprint {
        MusicalFingerprint {
            tone: None,
            key: None,
            key_claim: None,
            intonation: None,
            groove: None,
        }
    }

    /// Internally consistent flat-only intonation deficit: mean −20 ¢ (E1
    /// fires at ≤ −15) while mean_abs (20 < 25) and in_tune_ratio (0.55 ≥
    /// 0.5) stay on the healthy side of E3/E2.
    fn flat_sustains_fingerprint() -> MusicalFingerprint {
        let mut fp = healthy_fingerprint();
        fp.intonation = Some(theory::IntonationSummary {
            note_count: 20,
            mean_cents: -20.0,
            mean_abs_cents: 20.0,
            in_tune_ratio: 0.55,
            tendencies: Vec::new(),
        });
        fp
    }

    // AC1 E1: fires at the −15 ¢ threshold, not just inside it, and never
    // below the 12-note evidence bar (a persisted blob could carry one).
    #[test]
    fn s2_flat_mean_tags_pitch_sag_only_above_bar() {
        let mut fp = flat_sustains_fingerprint();
        fp.intonation.as_mut().unwrap().mean_cents = FLAT_SUSTAIN_MEAN_CENTS; // −15.0 exactly
        assert_eq!(evidence_tags(&fp), vec!["pitch-sag-sustain"]);

        fp.intonation.as_mut().unwrap().mean_cents = -14.9;
        assert!(
            evidence_tags(&fp).is_empty(),
            "just inside tolerance → no tag"
        );

        fp.intonation.as_mut().unwrap().mean_cents = -40.0;
        fp.intonation.as_mut().unwrap().note_count = MIN_INTONATION_NOTES - 1;
        assert!(
            evidence_tags(&fp).is_empty(),
            "below the note-count evidence bar even a big sag derives nothing"
        );
    }

    // AC1 E2: majority-off-center fires strictly below 0.5, above the bar.
    #[test]
    fn s2_low_in_tune_ratio_tags_pitch_drift() {
        let mut fp = healthy_fingerprint();
        fp.intonation.as_mut().unwrap().in_tune_ratio = 0.49;
        assert_eq!(evidence_tags(&fp), vec!["pitch-drift"]);

        fp.intonation.as_mut().unwrap().in_tune_ratio = DRIFT_MAX_IN_TUNE_RATIO; // 0.5
        assert!(
            evidence_tags(&fp).is_empty(),
            "exactly half in tune → no tag"
        );

        fp.intonation.as_mut().unwrap().in_tune_ratio = 0.1;
        fp.intonation.as_mut().unwrap().note_count = MIN_INTONATION_NOTES - 1;
        assert!(
            evidence_tags(&fp).is_empty(),
            "below the note-count bar → silent"
        );
    }

    // AC1 E3: quarter-semitone mean absolute error fires at 25 ¢ exactly.
    #[test]
    fn s2_high_abs_error_tags_interval_accuracy() {
        let mut fp = healthy_fingerprint();
        fp.intonation.as_mut().unwrap().mean_abs_cents = INACCURATE_MEAN_ABS_CENTS; // 25.0
        assert_eq!(evidence_tags(&fp), vec!["interval-accuracy"]);

        fp.intonation.as_mut().unwrap().mean_abs_cents = 24.9;
        assert!(evidence_tags(&fp).is_empty());
    }

    // AC1 E4/E5 + the graduation edge: 0.69 is uneven; 0.49 is uneven AND
    // unstable (E5 adds to E4, never replaces it); 0.7 is clean; and below
    // the onset bar even chaos derives nothing.
    #[test]
    fn s2_uneven_timing_tags_graduate_with_severity() {
        let mut fp = healthy_fingerprint();
        fp.groove.as_mut().unwrap().timing_consistency = 0.69;
        assert_eq!(evidence_tags(&fp), vec!["uneven-eighths"]);

        fp.groove.as_mut().unwrap().timing_consistency = 0.49;
        assert_eq!(
            evidence_tags(&fp),
            vec!["uneven-eighths", "tempo-instability"]
        );

        fp.groove.as_mut().unwrap().timing_consistency = UNEVEN_TIMING_CONSISTENCY; // 0.7
        assert!(
            evidence_tags(&fp).is_empty(),
            "at the dial → steady enough, no tag"
        );

        fp.groove.as_mut().unwrap().timing_consistency = 0.2;
        fp.groove.as_mut().unwrap().onset_count = MIN_GROOVE_ONSETS - 1;
        assert!(
            evidence_tags(&fp).is_empty(),
            "below the onset bar → silent"
        );
    }

    // AC1 E6: a noise-dominated tone derives BOTH family readings of the one
    // measurement — family filtering later keeps the applicable one.
    #[test]
    fn s2_noisy_tone_tags_breath_and_scratch() {
        let mut fp = healthy_fingerprint();
        fp.tone.as_mut().unwrap().air_noise = NOISY_TONE_AIR_NOISE; // 0.6 exactly
        assert_eq!(evidence_tags(&fp), vec!["breathy-onset", "scratchy-tone"]);

        fp.tone.as_mut().unwrap().air_noise = 0.59;
        assert!(evidence_tags(&fp).is_empty());
    }

    // AC1 E7: an unfocused tonal core reads as the corpus's fundamental
    // tone-production tag, at 0.35 exactly and not above.
    #[test]
    fn s2_unfocused_core_tags_tone_inconsistency() {
        let mut fp = healthy_fingerprint();
        fp.tone.as_mut().unwrap().core_clarity = UNFOCUSED_CORE_CLARITY; // 0.35 exactly
        assert_eq!(evidence_tags(&fp), vec!["tone-inconsistency"]);

        fp.tone.as_mut().unwrap().core_clarity = 0.36;
        assert!(evidence_tags(&fp).is_empty());
    }

    // AC2: healthy measurements and no measurements both select nothing, for
    // every family — the engine has no filler path.
    #[test]
    fn s2_healthy_fingerprint_selects_nothing() {
        for fp in [healthy_fingerprint(), empty_fingerprint()] {
            assert!(evidence_tags(&fp).is_empty());
            for family in ["Brass", "Strings", "Voice", "Woodwind", "Keyboard"] {
                assert_eq!(
                    select_pedagogy(family, &fp),
                    None,
                    "no measured deficit must mean no tip for {family}"
                );
            }
        }
    }

    // AC3: even a worst-on-every-axis fingerprint derives ONLY the tags that
    // have a measured evidence source. Attack-quality tags and directional
    // rushing are unmeasured and must stay unreachable.
    #[test]
    fn s2_derived_tags_stay_inside_the_measured_set() {
        let mut fp = healthy_fingerprint();
        fp.intonation = Some(theory::IntonationSummary {
            note_count: 30,
            mean_cents: -40.0,
            mean_abs_cents: 45.0,
            in_tune_ratio: 0.2,
            tendencies: Vec::new(),
        });
        fp.groove.as_mut().unwrap().timing_consistency = 0.1;
        fp.tone.as_mut().unwrap().air_noise = 0.9;
        fp.tone.as_mut().unwrap().core_clarity = 0.1;

        let tags = evidence_tags(&fp);
        assert_eq!(
            tags,
            vec![
                "pitch-sag-sustain",
                "pitch-drift",
                "interval-accuracy",
                "uneven-eighths",
                "tempo-instability",
                "breathy-onset",
                "scratchy-tone",
                "tone-inconsistency",
            ],
            "the full derivable set, in deterministic E1..E7 order"
        );
        for unmeasured in [
            "cracked-attacks",
            "attack-clarity",
            "ascending-leap-attacks",
            "rushing-runs",
            "tempo-drag-on-articulation",
        ] {
            assert!(
                !tags.contains(&unmeasured),
                "{unmeasured} has no measured evidence source and must never be derived"
            );
        }
    }

    // AC4: the player's family bounds the answer — a brass player's flat
    // sustains get Schlossberg's long tones, never a Suzuki entry; a family
    // with no entry for the evidence stays silent.
    #[test]
    fn s2_selection_filters_by_family() {
        let fp = flat_sustains_fingerprint();

        let brass = select_pedagogy("Brass", &fp).expect("brass has long-tone pedagogy");
        assert_eq!(brass.id, "brass-schlossberg-long-tones");
        assert_eq!(brass.family, Family::Brass);

        assert_eq!(
            select_pedagogy("Strings", &fp),
            None,
            "no strings entry carries pitch-sag-sustain — silence, not a borrowed family's tip"
        );

        // Whatever family answers, the entry belongs to it.
        for family in ["Brass", "Strings", "Voice", "Woodwind", "Keyboard"] {
            if let Some(entry) = select_pedagogy(family, &fp) {
                assert_eq!(entry.family, Family::from_display_name(family).unwrap());
            }
        }
    }

    // AC4: unknown, empty, and wrong-cased family strings select nothing —
    // the display-name contract is exact.
    #[test]
    fn s2_unknown_family_selects_nothing() {
        let mut fp = flat_sustains_fingerprint();
        fp.groove.as_mut().unwrap().timing_consistency = 0.1; // plenty of evidence
        for family in ["", "Percussion", "brass", "BRASS", "Theremin"] {
            assert_eq!(select_pedagogy(family, &fp), None, "family {family:?}");
        }
    }

    // AC5: on a fixed-pitch family measured cents are the instrument's
    // tuning, so intonation tags (E1–E3) never reach matching — an
    // out-of-tune keyboard alone selects nothing, and with uneven timing it
    // selects timing pedagogy via E4 only.
    #[test]
    fn s2_fixed_pitch_family_gets_no_intonation_tags() {
        let mut fp = healthy_fingerprint();
        fp.intonation = Some(theory::IntonationSummary {
            note_count: 30,
            mean_cents: -30.0,
            mean_abs_cents: 30.0,
            in_tune_ratio: 0.3,
            tendencies: Vec::new(),
        });
        assert_eq!(
            select_pedagogy("Keyboard", &fp),
            None,
            "an out-of-tune piano is the tuner's problem, not the player's"
        );
        // The same evidence on a bendable-pitch family DOES select.
        assert!(select_pedagogy("Brass", &fp).is_some());

        fp.groove.as_mut().unwrap().timing_consistency = 0.6; // E4 only
        let entry = select_pedagogy("Keyboard", &fp).expect("timing evidence applies to keyboard");
        assert_eq!(
            entry.id, "keyboard-hanon-evenness",
            "selected via uneven-eighths, never via the suppressed pitch tags"
        );
    }

    // AC6 (core rule): equal overlap resolves to the lexicographically
    // smallest id — pinned with tags the fingerprint cannot yet produce, via
    // the exposed matching core.
    #[test]
    fn s2_tie_breaks_by_smallest_id() {
        let corpus = load_corpus();
        // Both brass-arban-attack-tu and brass-arban-single-tonguing carry
        // attack-clarity (overlap 1 each).
        let entry = select_entry(&corpus, Family::Brass, &["attack-clarity"])
            .expect("two brass entries match");
        assert_eq!(entry.id, "brass-arban-attack-tu");
    }

    // AC6 (end to end): a real tie in the shipped corpus — a breathy voice
    // fingerprint matches Concone and García at overlap 1 — resolves the
    // same way on every call.
    #[test]
    fn s2_selection_is_deterministic_end_to_end() {
        let mut fp = healthy_fingerprint();
        fp.tone.as_mut().unwrap().air_noise = 0.7;
        let first = select_pedagogy("Voice", &fp).expect("voice has breathiness pedagogy");
        assert_eq!(first.id, "voice-concone-legato", "smallest id wins the tie");
        for _ in 0..3 {
            assert_eq!(select_pedagogy("Voice", &fp).as_ref(), Some(&first));
        }
    }

    // AC6: more trigger overlap beats a smaller id — drifting AND inaccurate
    // pitch (overlap 2) selects Suzuki's listening entry over any overlap-1
    // strings entry.
    #[test]
    fn s2_most_trigger_overlap_wins() {
        let mut fp = healthy_fingerprint();
        fp.intonation = Some(theory::IntonationSummary {
            note_count: 20,
            mean_cents: -5.0,
            mean_abs_cents: 30.0,
            in_tune_ratio: 0.3,
            tendencies: Vec::new(),
        });
        assert_eq!(
            evidence_tags(&fp),
            vec!["pitch-drift", "interval-accuracy"],
            "fixture sanity: exactly the two listening triggers"
        );
        let entry = select_pedagogy("Strings", &fp).expect("strings intonation pedagogy exists");
        assert_eq!(entry.id, "strings-suzuki-listening");
    }

    // MF1 (review round 1): the fixed-pitch exemption pinned AT THE SEAM.
    // No Keyboard corpus entry carries an E1–E3 trigger today, so the
    // end-to-end AC5 test would stay green if the strip were deleted — S4
    // corpus growth (data-only) must never be this path's only guard. This
    // asserts directly that "Keyboard" strips exactly the intonation-derived
    // tags from an out-of-tune fingerprint while "Brass" keeps them.
    #[test]
    fn s2_fixed_pitch_exemption_strips_intonation_tags_at_the_seam() {
        let mut fp = healthy_fingerprint();
        fp.intonation = Some(theory::IntonationSummary {
            note_count: 30,
            mean_cents: -30.0,
            mean_abs_cents: 30.0,
            in_tune_ratio: 0.3,
            tendencies: Vec::new(),
        });
        fp.groove.as_mut().unwrap().timing_consistency = 0.6; // E4 rides along

        assert_eq!(
            evidence_tags_for_family("Brass", &fp),
            vec![
                "pitch-sag-sustain",
                "pitch-drift",
                "interval-accuracy",
                "uneven-eighths",
            ],
            "a bendable-pitch family keeps every measured tag"
        );
        let keyboard = evidence_tags_for_family("Keyboard", &fp);
        assert_eq!(
            keyboard,
            vec!["uneven-eighths"],
            "Keyboard strips exactly the intonation-derived tags — cents \
             there are the instrument's tuning, not the player's technique"
        );
        for tag in INTONATION_DEFICIT_TAGS {
            assert!(
                !keyboard.contains(&tag),
                "{tag} must never survive the fixed-pitch strip"
            );
        }
    }

    // SF2 (review round 1): every tag the engine can emit is a member of the
    // corpus's trigger vocabulary — a data-only S4 rename of a trigger must
    // go red here, never silently orphan an engine tag into unselectability.
    #[test]
    fn s2_engine_tags_are_corpus_vocabulary() {
        // Worst-on-every-axis fingerprint emits the full E1..E7 set.
        let mut fp = healthy_fingerprint();
        fp.intonation = Some(theory::IntonationSummary {
            note_count: 30,
            mean_cents: -40.0,
            mean_abs_cents: 45.0,
            in_tune_ratio: 0.2,
            tendencies: Vec::new(),
        });
        fp.groove.as_mut().unwrap().timing_consistency = 0.1;
        fp.tone.as_mut().unwrap().air_noise = 0.9;
        fp.tone.as_mut().unwrap().core_clarity = 0.1;
        let emitted = evidence_tags(&fp);
        assert_eq!(emitted.len(), 8, "fixture sanity: the full emission");

        let corpus = load_corpus();
        let vocabulary: HashSet<&str> = corpus
            .iter()
            .flat_map(|e| e.triggers.iter().map(String::as_str))
            .collect();
        for tag in emitted {
            assert!(
                vocabulary.contains(tag),
                "engine tag {tag} is not in the corpus trigger vocabulary — \
                 an S4 rename must update the mapping, not orphan the tag"
            );
        }
    }

    // SF2 bonus: the two engine tags no other test drove through a real
    // selection do select — tempo-instability is Czerny's ONLY matching
    // trigger on Keyboard (the tie with Hanon's uneven-eighths resolves to
    // the smaller id, so the tag decides czerny matches at all), and
    // scratchy-tone is the only string trigger a noisy tone can reach.
    #[test]
    fn s2_tempo_instability_and_scratchy_tone_select_for_real() {
        let mut fp = healthy_fingerprint();
        fp.groove.as_mut().unwrap().timing_consistency = 0.49; // E4 + E5
        let kb = select_pedagogy("Keyboard", &fp).expect("an unstable pulse selects keyboard work");
        assert_eq!(kb.id, "keyboard-czerny-velocity");
        assert!(
            kb.triggers.contains(&"tempo-instability".to_owned())
                && !kb.triggers.contains(&"uneven-eighths".to_owned()),
            "Czerny matched via tempo-instability alone: {:?}",
            kb.triggers
        );

        let mut fp = healthy_fingerprint();
        fp.tone.as_mut().unwrap().air_noise = 0.7; // E6
        let strings =
            select_pedagogy("Strings", &fp).expect("a noisy string tone selects pedagogy");
        assert_eq!(strings.id, "strings-suzuki-tonalization");
        assert!(
            strings.triggers.contains(&"scratchy-tone".to_owned()),
            "tonalization matched via scratchy-tone: {:?}",
            strings.triggers
        );
    }
}
