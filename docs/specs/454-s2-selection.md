# Spec: Evidence-gated pedagogy selection engine (#454 S2)

## 1. Summary
A pure, brain-side selection engine that turns a session's measured
[`MusicalFingerprint`] into corpus evidence tags and picks the single most
relevant method-book entry for the player's instrument family — plus a thin
`method_book_tip` IPC command that resolves the family from the last stored
session the same way the recap does. Evidence-gated end to end: no measured
deficit → no tip ("silence > lies"). No surfacing in this slice.

## 2. Problem / why
S1 (#458) shipped the corpus: 22 entries across 5 families, keyed by 34
kebab-case trigger tags. Nothing consumes it. The corpus's trigger vocabulary
is forward-looking; the fingerprint measures a much smaller set of things. The
S1 spec named the risk explicitly: *"S2 must reconcile it with the
fingerprint's actual evidence signals."* This slice is that reconciliation —
an audited mapping from **what is actually measured** to the tags the corpus
can answer, and a deterministic entry pick.

## 3. Non-goals
- No recap or coaching-box surfacing, no frontend — S3 wires the tip into
  #453's coaching box.
- No new measurements. Tags whose evidence the fingerprint cannot carry
  (attack quality, slur smearing, hands-together coordination, …) are **not**
  derived — see the "not mapped" audit below. Growing the measured set is
  future ears/brain work, not this slice.
- No corpus changes (S4 is data-only growth).
- No LLM involvement; the engine is pure and offline.

## 4. Contract / interface

### 4.1 The fingerprint → evidence-tag mapping (the audit)

What `MusicalFingerprint` actually measures, today:

| Dimension | Fields carried | Present when |
|---|---|---|
| `intonation: Option<IntonationSummary>` | `note_count`, `mean_cents`, `mean_abs_cents`, `in_tune_ratio`, `tendencies` | ≥ 12 notes (`coaching::aggregate_intonation`) |
| `groove: Option<GrooveDescriptor>` | `tempo_bpm`, `swing_ratio`, `mean_ioi_secs`, `timing_consistency`, `onset_count` | ≥ 6 onsets (`coaching::aggregate_groove`) |
| `tone: Option<ToneDescriptor>` | `brightness`, `warmth`, `air_noise`, `core_clarity`, `vibrato_quality` (all 0..1 session means) | ≥ 1 toned phrase |
| `key` / `key_claim` | key estimate + claim strength | confident fit |

**Mapping table** (`brain::pedagogy::evidence_tags`). A row fires only when
its dimension is present, its count bar is met, and its threshold is crossed.

| # | Measured field | Fires when | Count bar | Evidence tag(s) | Rationale |
|---|---|---|---|---|---|
| E1 | `intonation.mean_cents` | `≤ −15.0` | `note_count ≥ 12` | `pitch-sag-sustain` | Session mean a full in-tune tolerance (`DEFAULT_IN_TUNE_TOLERANCE_CENTS` = 15) **flat**: sustained pitch is sagging below center. Signed mean, so direction is measured, not guessed. |
| E2 | `intonation.in_tune_ratio` | `< 0.5` | `note_count ≥ 12` | `pitch-drift` | The majority of observed notes landed outside ±15 cents — pitch is drifting off center. |
| E3 | `intonation.mean_abs_cents` | `≥ 25.0` | `note_count ≥ 12` | `interval-accuracy` | Average pitch-placement error of a quarter semitone or worse; intervals between successive notes cannot be landing accurately. |
| E4 | `groove.timing_consistency` | `< 0.7` | `onset_count ≥ 6` | `uneven-eighths` | Below 0.7 is exactly the band `coaching::describe_groove` already reads to the player as "uneven" — same dial, same meaning. |
| E5 | `groove.timing_consistency` | `< 0.5` | `onset_count ≥ 6` | `tempo-instability` (in addition to E4) | IOI coefficient of variation above 0.5: not just uneven subdivisions, the pulse itself is unstable. |
| E6 | `tone.air_noise` | `≥ 0.6` | tone measured | `breathy-onset`, `scratchy-tone` | Session-mean noise component dominates the sound. One measurement, two family readings — voice entries carry `breathy-onset`, strings entries carry `scratchy-tone`; family filtering (4.2) picks the one that applies. |
| E7 | `tone.core_clarity` | `≤ 0.35` | tone measured | `tone-inconsistency` | The tonal core failed to hold focus across the session; `tone-inconsistency` is the corpus's fundamental tone-production tag (tonalization / sonorité / mouthpiece work). |

Count bars deliberately **mirror the upstream evidence gates** (the fingerprint
module's doc: "Building code must reuse those gates — do not loosen them").
They re-check here because fingerprints also arrive from persistence, where a
hand-written or legacy blob could carry a dimension the live gates would have
withheld.

**Audited and NOT mapped** (silence > lies — each of these is either not a
deficit, not measured, or not attributable to the player):

| Signal / tag family | Why not |
|---|---|
| `key`, `key_claim` | Key detection is display honesty only (RV philosophy) — never a deficit, never coaching evidence. |
| `groove.swing_ratio` | Unequal IOI pairs may be deliberate swing; the fingerprint cannot distinguish intent, so no `uneven-eighths` from swing. |
| `rushing-runs`, `tempo-drag-on-articulation` | "Rushing"/"dragging" claim a *direction* of tempo drift; `timing_consistency` is an undirected CV. No directional tempo measurement exists → no tag. |
| `groove.tempo_bpm`, `mean_ioi_secs` | Facts, not deficits. |
| `tone.brightness`, `tone.warmth` | Aesthetic axes, not deficits. |
| `tone.vibrato_quality` | Measured, but no corpus trigger tag exists yet (an S4 data-only add can claim it later). |
| Attack/onset-quality tags (`cracked-attacks`, `attack-clarity`, `soft-attack-smear`, `repeated-note-smear`, `ascending-leap-attacks`, `scooped-onsets`, `smeared-slurs`, `register-shift-breaks`, …) | The fingerprint carries **no attack/onset-quality measurement**. These 12 of the corpus's 34 tags stay unreachable until ears/brain measure attacks. |
| Session-shape tags (`accuracy-collapse-at-tempo`, `hands-together-breakdown`, `endurance-fade`, `phrase-end-collapse`, `new-material-overreach`, …) | Would need within-session trajectory or score-alignment evidence the fingerprint does not carry. |

**Fixed-pitch honesty (#417-4/#389):** for families where the player cannot
bend pitch (`coaching::fixed_pitch_family` — "Keyboard", "Percussion"),
measured cents are the *instrument's* tuning, not the player's technique.
Intonation-derived tags (E1–E3) are dropped before matching for those
families, reusing `fixed_pitch_family` (never a duplicate list).

### 4.2 Selection (pure, brain-side)

```rust
// crates/brain/src/pedagogy.rs (S2 additions)
impl Family {
    /// Exact display-name parse ("Brass" … "Keyboard") — the casing
    /// `instrument_family_for` returns over IPC. Unknown/empty → None.
    pub fn from_display_name(name: &str) -> Option<Family>;
}

/// The corpus evidence tags derivable from what `fingerprint` measured.
/// Deterministic order (E1..E7). Empty when nothing crossed a bar.
pub fn evidence_tags(fingerprint: &MusicalFingerprint) -> Vec<&'static str>;

/// Deterministic matching core over any entry set (exposed so tests can
/// exercise tie-breaks with tags the fingerprint cannot yet produce).
pub fn select_entry<'a>(
    entries: &'a [PedagogyEntry],
    family: Family,
    tags: &[&str],
) -> Option<&'a PedagogyEntry>;

/// The engine: family display name + measured fingerprint → the one most
/// relevant corpus entry, or None below the evidence bars.
pub fn select_pedagogy(family: &str, fingerprint: &MusicalFingerprint)
    -> Option<PedagogyEntry>;
```

Matching rule (deterministic): entries of the player's family whose `triggers`
intersect the evidence tags; the pick is **most trigger overlap, ties broken
by lexicographically smallest entry id**. No family match / no tags / unknown
family → `None`.

### 4.3 IPC command (thin wrapper, no surfacing)

```rust
// apps/desktop/src-tauri/src/commands.rs (near practice_suggestions)
pub struct PedagogyTipDto {
    pub topic: String,
    pub guidance: String,
    /// "{author}, {title}" — attribution is ALWAYS present.
    pub source_line: String,
}
pub fn method_book_tip_impl(state: &AppState) -> Option<PedagogyTipDto>;
#[tauri::command] pub fn method_book_tip(...) -> Option<PedagogyTipDto>;
```

The command reads the **most recent stored session** (`list_recent(1)` +
`load_recap`), resolves the family from the recap's instrument via
`instrument_family_for` — the same resolution the recap itself used — and runs
`select_pedagogy` over that recap's fingerprint. `None` is the calm, common
answer: no sessions, no fingerprint on the latest session, unknown
instrument, no matching entry, or a store error (warn + `None`, never an
error surface — the `my_patterns`/`practice_suggestions` discipline).

## 5. Acceptance criteria (numbered, testable)
1. **Each mapping fires only above its bar.** For every row E1–E7: a
   synthetic fingerprint at the threshold yields exactly that row's tag(s); a
   fingerprint just inside the healthy side yields none; for E1–E5, a
   deficit-valued fingerprint below the count bar (`note_count < 12` /
   `onset_count < 6`) yields none.
2. **Healthy → None.** A fully measured, healthy fingerprint yields no
   evidence tags and `select_pedagogy` returns `None`; an empty fingerprint
   likewise.
3. **Unmeasured stays silent.** Even a worst-on-every-axis fingerprint never
   derives a tag outside the E1–E7 set (attack tags, `rushing-runs`, … are
   unreachable).
4. **Family filtering.** A returned entry's family always equals the player's
   parsed family; a deficit measurable on several families resolves to the
   *player's* family entry (a brass player's flat sustains select
   Schlossberg's long tones, never a Suzuki entry); a family with no entry
   for the evidence yields `None`; unknown/empty/wrong-cased family strings
   yield `None`.
5. **Fixed-pitch honesty.** On "Keyboard", intonation-derived tags (E1–E3)
   never fire: an out-of-tune keyboard fingerprint alone selects nothing, and
   with uneven timing added it selects a keyboard timing entry via E4 only.
6. **Determinism + tie-break.** The same inputs always select the same entry;
   with equal trigger overlap the lexicographically smallest id wins (both at
   the `select_entry` core and end-to-end through a real tie in the shipped
   corpus).
7. **Attribution always present.** Every tip the command returns carries
   `source_line == "{author}, {title}"` with both parts nonempty.
8. **Command honesty.** `method_book_tip_impl` returns `None` when the store
   has no sessions, when the latest session's recap has no fingerprint (even
   if an older session was measured — the tip speaks to the *last* session),
   and when the instrument resolves to no family; it never errors.

## 6. Edge cases & failure modes
- Fingerprint dimension present but count below bar (persisted/legacy blob) →
  that dimension derives nothing (AC1).
- `timing_consistency < 0.5` → E4 **and** E5 (graduated, not exclusive).
- Unknown family string, empty string (unknown instrument), lowercase
  `"brass"` → `None` (exact display-name contract; `instrument_family_for`
  only ever emits exact casing or "").
- "Percussion" (a fixed-pitch family with no corpus file) → parse fails →
  `None`, and never via intonation tags.
- Store read failure in the command → `tracing::warn!` + `None`.
- Latest session ordering: `list_recent` is newest-first by `started_at`; the
  command consumes index 0 only.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 E1 | `pedagogy::tests::s2_flat_mean_tags_pitch_sag_only_above_bar` | −15.0 → `pitch-sag-sustain`; −14.9 → no tags; −40 with `note_count` 11 → no tags |
| AC1 E2 | `pedagogy::tests::s2_low_in_tune_ratio_tags_pitch_drift` | 0.49 → `pitch-drift`; 0.5 → none; 0.1 with 11 notes → none |
| AC1 E3 | `pedagogy::tests::s2_high_abs_error_tags_interval_accuracy` | 25.0 → `interval-accuracy`; 24.9 → none |
| AC1 E4/E5 + edge | `pedagogy::tests::s2_uneven_timing_tags_graduate_with_severity` | 0.69 → `uneven-eighths` only; 0.49 → + `tempo-instability`; 0.7 → none; 0.2 with 5 onsets → none |
| AC1 E6 | `pedagogy::tests::s2_noisy_tone_tags_breath_and_scratch` | 0.6 → both tags; 0.59 → none |
| AC1 E7 | `pedagogy::tests::s2_unfocused_core_tags_tone_inconsistency` | 0.35 → tag; 0.36 → none |
| AC2 | `pedagogy::tests::s2_healthy_fingerprint_selects_nothing` | healthy + empty fingerprints → no tags, `select_pedagogy` None for every family |
| AC3 | `pedagogy::tests::s2_derived_tags_stay_inside_the_measured_set` | worst-everything fingerprint derives ⊆ {E1–E7 tags}; attack tags & `rushing-runs` absent |
| AC4 | `pedagogy::tests::s2_selection_filters_by_family` | flat-sustain fp: "Brass" → `brass-schlossberg-long-tones`; "Strings" → None; returned entry family always matches |
| AC4 | `pedagogy::tests::s2_unknown_family_selects_nothing` | ""/"Percussion"/"brass" → None |
| AC5 | `pedagogy::tests::s2_fixed_pitch_family_gets_no_intonation_tags` | out-of-tune-only fp + "Keyboard" → None; + timing 0.6 → `keyboard-hanon-evenness` |
| AC6 core | `pedagogy::tests::s2_tie_breaks_by_smallest_id` | `select_entry`, Brass, `{attack-clarity}` → `brass-arban-attack-tu` (beats `…single-tonguing`) |
| AC6 e2e | `pedagogy::tests::s2_selection_is_deterministic_end_to_end` | breathy fp + "Voice": overlap-1 tie resolves to `voice-concone-legato`; repeated calls identical |
| AC6 overlap | `pedagogy::tests::s2_most_trigger_overlap_wins` | drift+inaccuracy fp + "Strings" → `strings-suzuki-listening` (overlap 2 beats 1) |
| AC7 + AC8 | `commands::tests::method_book_tip_cites_book_or_stays_silent` | empty store → None; measured Trumpet session → tip with topic + `source_line` "Max Schlossberg, Daily Drills and Technical Studies"; fingerprint-less newer session → None; unknown instrument → None |

## 8. Architecture / approach
All selection logic lives in `crates/brain/src/pedagogy.rs` beside the S1
loader — pure functions over the embedded corpus, zero I/O, zero network,
nothing near the audio thread. The command layer adds one DTO + one
`#[tauri::command]` beside `practice_suggestions` (deliberately minimal
`commands.rs` footprint — a concurrent branch is editing that file) and one
registration line in `main.rs`. Threshold constants are named, doc-commented
with their rationale, and pinned by tests; the two count bars carry comments
naming the `coaching.rs` gates they mirror.

## 9. Slice breakdown
| # | Slice (goal) | Footprint | Depends on |
|---|---|---|---|
| S2 (this) | evidence-tag derivation + deterministic selection + `method_book_tip` command | `crates/brain/src/pedagogy.rs`, `commands.rs` (one DTO + command), `main.rs` (one line), this spec | S1 (#458) |
| S3 | recap + coaching-box surfacing with attribution | commands.rs, #453 box, frontend | S2, #453 |
| S4 | corpus growth per instrument (data-only) — may claim `vibrato_quality` etc. with new tags | `pedagogy/*.json` | S1 |

## 10. Risks / open questions
- Only 7 of the corpus's 34 tags are currently reachable — honest, but it
  means whole entries (all attack/articulation pedagogy) cannot surface yet.
  That is a *measurement* gap, tracked as future ears/brain work, not a
  selection bug; the mapping table is the single place to extend.
- Thresholds are judgment calls anchored to existing dials
  (`DEFAULT_IN_TUNE_TOLERANCE_CENTS`, `describe_groove`'s 0.7 "uneven" band).
  They are constants with tests pinning behavior at the boundary, so tuning
  them later is a visible, reviewed change.
- Tie-break by id is arbitrary but stable; if pedagogy ever wants priority
  ordering, that's a corpus field (data-only PR), not a code change.

## 11. References
- Issue #454; S1 spec `docs/specs/454-s1-corpus.md`; S1 PR #458.
- `crates/brain/src/fingerprint.rs` (the measured contract + "reuse the
  gates" rule), `crates/brain/src/coaching.rs` (`aggregate_intonation`
  MIN_NOTES=12, `aggregate_groove` MIN_ONSETS=6, `describe_groove`,
  `fixed_pitch_family`).
- `crates/theory/src/intonation.rs` (`DEFAULT_IN_TUNE_TOLERANCE_CENTS`),
  `crates/groove/src/analyze.rs` (`timing_consistency` = 1 − CV).
- `apps/desktop/src-tauri/src/commands.rs` (`instrument_family_for`,
  `practice_suggestions` discipline), #453 S1 (PR #459).
