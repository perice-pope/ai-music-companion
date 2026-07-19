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
//! Nothing consumes the corpus yet: selection (S2) and surfacing (S3, behind
//! #453's coaching box) build on this seam.

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
}
