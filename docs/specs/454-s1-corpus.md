# Spec: Pedagogy corpus + pd/paraphrase-only CI gate (#454 S1)

## 1. Summary
A method-book pedagogy corpus shipped in the repo (`pedagogy/*.json`, one data file per
instrument family, like `profiles/` is one file per instrument), a `brain` loader
(`crates/brain/src/pedagogy.rs`), and a CI gate that refuses verbatim text in entries
sourced from in-copyright books. Content only — no surfacing (that's S3, behind #453's
coaching box) and no selection engine (S2).

## 2. Problem / why
#454: the coach should give instrument-specific technique instruction grounded in the
canonical method books (Arban, Schlossberg, Suzuki, Hanon, …). Today the
`instrument_family` seam (#417-4/#389, `instrument_family_for` in
`apps/desktop/src-tauri/src/commands.rs`) only routes vocabulary. Before any surfacing
can be built we need (a) the corpus itself and (b) the copyright review gate the issue
mandates, so corpus-growth PRs (S4) are data-only and safe by construction.

**Copyright house rule (from the issue, verbatim policy):**
- **`pd`** (public domain — Arban 1864, Hanon 1873, Czerny, García, Marchesi, Klosé,
  Taffanel & Gaubert 1923…): may carry verbatim quotes, exercise text, notation.
- **`paraphrase-only`** (in copyright — Schlossberg 1937, Suzuki, Moyse 1934,
  Lamperti/Brown 1931…): attributed paraphrase of technique *facts* only, plus a
  "see [book], section N" pointer to the player's own copy. Never verbatim passages,
  never reproduced exercises. CI enforces this.

## 3. Non-goals
- No selection engine (evidence tag → entry matching) — S2.
- No recap or coaching-box surfacing, no IPC command, no frontend — S3.
- No per-instrument depth beyond the seed set — S4 is data-only growth PRs.
- No trademark-implying marketing copy anywhere ("Suzuki training" etc.).

## 4. Contract / interface
`pedagogy/<family>.json` — five files: `brass.json`, `strings.json`, `voice.json`,
`woodwind.json`, `keyboard.json`. Each is a JSON array of entries:

```json
{
  "id": "brass-arban-interval-bottom-note",
  "family": "Brass",
  "topic": "Ascending interval leaps",
  "source": {
    "title": "Complete Conservatory Method for Trumpet",
    "author": "Jean-Baptiste Arban",
    "year": 1864,
    "status": "pd",
    "section": "Interval studies"
  },
  "guidance": "…paraphrase or PD quote…",
  "quote": "…optional verbatim quote — pd entries only…",
  "triggers": ["ascending-leap-attacks", "cracked-attacks"]
}
```

- `family` ∈ `Brass | Strings | Voice | Woodwind | Keyboard` (the display-name casing
  `instrument_family_for` already returns over IPC).
- `source.status` ∈ `"pd" | "paraphrase-only"`.
- `quote` is optional and **forbidden** when `status == "paraphrase-only"`.
- `triggers`: nonempty list of nonempty kebab-case evidence tags (the S2 seam), e.g.
  `ascending-leap-attacks`, `uneven-eighths`, `pitch-sag-sustain`.

Rust (crates/brain/src/pedagogy.rs):
```rust
pub enum Family { Brass, Strings, Voice, Woodwind, Keyboard }
pub enum SourceStatus { Pd, ParaphraseOnly }
pub struct SourceRef { title, author, year, status, section }
pub struct PedagogyEntry { id, family, topic, source, guidance, quote: Option<String>, triggers: Vec<String> }
fn load_corpus() -> Vec<PedagogyEntry>;                          // embedded, infallible after CI; private since #508
pub fn try_load_corpus() -> Result<Vec<PedagogyEntry>, PedagogyError>;
pub fn validate_entries(&[PedagogyEntry]) -> Result<(), PedagogyError>;
```

**Shipping mechanism (documented decision):** the corpus is embedded at compile time via
`include_str!("../../../pedagogy/<family>.json")`, the same pattern the `idiom` crate
uses for its seed corpus (`crates/idiom/src/corpus.rs`). `profiles/` ships as runtime
files because users' instrument catalogs are enumerated from disk and the packaged app
needed resource-dir resolution (#112); the pedagogy corpus has no runtime-file
requirement, so the simplest robust choice is compile-time embedding — no bundler
resource config, no missing-file failure mode, cargo re-builds when a JSON changes.
If S4 ever needs hot-reloadable corpus files, `try_load_corpus` already parses from
embedded strings and can grow a `from_dir` twin then.

## 5. Acceptance criteria (numbered, testable)
1. `try_load_corpus()` returns Ok, and every entry passes schema validation (nonempty
   id/topic/guidance/source fields, valid family, valid status, nonempty triggers with
   every tag nonempty, unique ids).
2. All five families are represented, with at least: Brass 6, Strings 4, Voice 3,
   Woodwind 3, Keyboard 3 entries.
3. The gate rejects a `paraphrase-only` entry carrying a `quote` field
   (`PedagogyError::QuoteFieldInParaphraseOnly`).
4. The gate rejects a `paraphrase-only` entry whose `guidance` contains a quoted run of
   more than 15 consecutive words (`PedagogyError::VerbatimInParaphraseOnly`); a quoted
   run of exactly 15 words passes (boundary), and curly quotes (“…”) are caught the same
   as ASCII quotes.
5. A `pd` entry may carry a `quote` field and long quoted runs in `guidance` — the gate
   accepts it.
6. The shipped corpus itself passes the gate (i.e. AC3/AC4 violations are impossible to
   merge: the gate test runs on the real embedded data in `cargo test`, which CI runs).
7. Every `paraphrase-only` entry's guidance names its source in the copy (attribution is
   in the text, founder's voice), enforced by a test that the source title or author
   appears in the guidance string.

## 6. Edge cases & failure modes
- Malformed JSON in a pedagogy file → `try_load_corpus` returns `PedagogyError::Parse`
  naming the file; `load_corpus` panics with that message (unreachable once CI is green
  — documented on the fn).
- Unknown `family` or `status` string → serde parse error (enums are closed).
- Unknown JSON keys → rejected (`deny_unknown_fields`), so a stray `quote`-like field
  (`excerpt`, `verbatim`) can't sneak content past the gate unvalidated.
- Duplicate ids across files → `PedagogyError::DuplicateId`.
- Empty trigger tag (`""` or whitespace) → `PedagogyError::EmptyTrigger`.
- Unbalanced quote mark in guidance → the scanner treats text after an unclosed opening
  quote up to end-of-string as a quoted span (conservative: over-catches, never
  under-catches).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `pedagogy::tests::corpus_loads_and_validates` | Ok + per-entry schema invariants |
| AC2 | `pedagogy::tests::family_coverage_meets_minimums` | 5 families, per-family minimum counts |
| AC3 | `pedagogy::tests::gate_rejects_quote_field_in_paraphrase_only` | planted fixture → `QuoteFieldInParaphraseOnly` |
| AC4 | `pedagogy::tests::gate_rejects_long_verbatim_in_paraphrase_only` | 16-word quoted run → `VerbatimInParaphraseOnly` |
| AC4 boundary | `pedagogy::tests::gate_allows_short_quotes_in_paraphrase_only` | 15-word quoted run → Ok |
| AC4 curly | `pedagogy::tests::gate_catches_curly_quotes` | “…16 words…” → `VerbatimInParaphraseOnly` |
| AC5 | `pedagogy::tests::pd_entries_may_quote` | pd fixture with quote field + long quoted guidance → Ok |
| AC6 | AC1's test runs on the real embedded corpus | shipped data is gate-clean |
| AC7 | `pedagogy::tests::paraphrase_entries_carry_attribution_in_copy` | title/author substring in guidance |
| dup id | `pedagogy::tests::gate_rejects_duplicate_ids` | `DuplicateId` |
| empty tag | `pedagogy::tests::gate_rejects_empty_trigger` | `EmptyTrigger` |
| unknown field | `pedagogy::tests::unknown_fields_rejected` | serde error on `excerpt` field |

## 8. Architecture / approach
New module `crates/brain/src/pedagogy.rs`, registered in `lib.rs`. Data in repo-root
`pedagogy/` (sibling of `profiles/` — same "adding content = adding a data file, no code"
convention). Zero network, zero runtime I/O. `load_corpus` is the module's internal
seam (private since #508 — external callers use `try_load_corpus`/`select_pedagogy`).

## 9. Slice breakdown
| # | Slice (goal) | Footprint | Depends on |
|---|---|---|---|
| S1 (this) | corpus format + gate + seed entries + loader | `pedagogy/`, `crates/brain/src/pedagogy.rs`, `lib.rs`, this spec | — |
| S2 | evidence-gated selection engine | `crates/brain/src/pedagogy.rs` (+selection) | S1, fingerprint evidence tags |
| S3 | recap + coaching-box surfacing with attribution | commands.rs, #453 box | S2, #453 |
| S4 | corpus growth per instrument (data-only PRs) | `pedagogy/*.json` | S1 |

## 10. Risks / open questions
- Trigger-tag vocabulary is forward-looking; S2 must reconcile it with the fingerprint's
  actual evidence signals. Tags are data, so renames are data-only PRs.
- The >15-word heuristic is a tripwire, not a lawyer: human review of `paraphrase-only`
  wording remains part of corpus-PR review. The gate exists so verbatim paste can't land
  silently.
- PD quotes are transcribed from memory of the standard editions; where confidence in
  exact wording is low, pd entries deliberately use close paraphrase instead of a
  `quote` field (pd status permits either — honesty about "verbatim" matters more than
  quota of quotes).

## 11. References
- Issue #454 (copyright rules are in the issue body), #453 (coaching box).
- `crates/idiom/src/corpus.rs` — the `include_str!` seed-corpus precedent.
- `profiles/` + `apps/desktop/src-tauri/src/commands.rs` `instrument_family_for` — the
  family seam and data-file convention.
- `docs/architecture/platform-spine-content-format.md`.
