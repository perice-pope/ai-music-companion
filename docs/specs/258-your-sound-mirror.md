# Spec: "Your sound" mirror — identity profile from fingerprints (#258)

> Part of epic #252. A read-mostly feature on top of **F2 (Learner Model)** and the existing
> `MusicalFingerprint` (migration 0004). It aggregates accumulated sessions into one evolving
> identity card and grounds the "you sound like…" line in #253's curated table. Builds behind F2.

## 1. Summary
The app quietly profiles the player from their accumulated `MusicalFingerprint`s — "you gravitate to
minor, swung, mid-register, bold — like a young [artist]" — and shows it as a single, evolving
identity card. The derivation is a **pure, deterministic aggregation** over the stored fingerprints
(reproducible from the same set); the real-world comparison is **grounded** (reuses #253's curated
grounding table) and **confidence-gated** ("still listening…" below threshold), with a clear
"keep playing to discover your sound" empty state before enough sessions exist.

## 2. Problem / why
The app already measures every session into a `MusicalFingerprint` (tone, key/mode, intonation,
groove — `apps/desktop/src/types/brain.ts`) and stores it (`sessions.fingerprint`, migration 0004),
but each fingerprint dies on its own recap screen. Nothing aggregates them into a stable picture of
*who the player is becoming*. People are addicted to learning about themselves; the cross-session
identity is the highest-retention surface in the epic and it is currently invisible. The recap's
per-session "Flavour" answers "what did I just play"; this answers "what kind of musician am I",
grounded in real music and honest about uncertainty.

## 3. Non-goals
- No new perception, pitch, or audio-thread work — this is **read/aggregate over already-stored
  fingerprints**, computed off the hot path (CLAUDE.md latency budget untouched).
- No LLM call of its own. The comparison comes from #253's curated `connections` table; any LLM
  rephrasing reuses #253's existing opt-in path and **must not invent** the artist/genre.
- Not a leaderboard, score, or grade — one descriptive card, no judgement, no ranking vs others.
- No history/timeline of how the profile changed over time (later slice if ever) — one current card.
- No write-back into the per-session fingerprints; `derive_sound_profile` only produces a snapshot.

## 4. Contract / interface

### Backend (Rust core, `brain::learner` — extends F2)
`derive_sound_profile` is the F2 transition declared in the epic (#252 §F2). It is **pure and
deterministic**: same fingerprint slice + same taste → identical `SoundProfile` (no clock, no RNG;
`now` is passed in only to stamp `derived_at`).

```rust
/// Aggregate ≥K stored fingerprints into the player's evolving identity.
/// Returns None below K sessions (the "keep playing" empty state).
pub fn derive_sound_profile(
    sessions: &[Fingerprint],     // brain::fingerprint::MusicalFingerprint, oldest→newest
    taste: &TasteProfile,         // stated genres/artists for grounding the comparison
) -> Option<SoundProfile>;

pub const MIN_SESSIONS: usize = 5;  // K — below this, None (empty state)

pub struct SoundProfile {
    pub sessions_counted: u32,            // how many fingerprints fed this snapshot
    pub mode_lean: Option<ModeLean>,      // minor vs major tendency
    pub feel: Option<Feel>,               // swung | straight | mixed
    pub register: Option<Register>,       // low | mid | high
    pub dynamics: Option<DynamicsLean>,   // soft | moderate | bold
    pub comparison: Option<Comparison>,   // grounded real-world "like a young X"
    pub confidence: f32,                  // 0..1 overall stability of the picture
    pub derived_at: Timestamp,
}

pub enum ModeLean { Minor, Major, Balanced }      // by fraction of minor-family modes
pub enum Feel { Swung, Straight, Mixed }          // by median groove.swing_ratio
pub enum Register { Low, Mid, High }              // by aggregated register band
pub enum DynamicsLean { Soft, Moderate, Bold }    // by aggregated dynamic level

pub struct Comparison {
    pub label: String,                    // "like a young Santana", "in a Bill Evans world"
    pub kind: ComparisonKind,             // Artist | Genre
    pub source: RevealSource,             // Grounded | LlmGrounded — reuses brain::connections
}
```

- `SoundProfile` is the `sound_profile: Option<SoundProfile>` field already reserved on
  `LearnerModel` (#252 §F2). It is a **derived snapshot**: callers run `derive_sound_profile` over
  the stored fingerprints and store the result on the model; it round-trips through F2's JSONB blob
  with the same forward-compat rules (unknown fields preserved).
- **Aggregation rules (deterministic, per axis, each independently gated):**
  - `mode_lean` ← over sessions whose `fingerprint.key` is present: count minor-family modes
    (aeolian, dorian, phrygian, locrian) vs major-family (ionian, lydian, mixolydian). `Minor` /
    `Major` when one side ≥ a defined fraction (e.g. ≥0.6), else `Balanced`. `None` if no session
    carried a confident `key`.
  - `feel` ← median of present `fingerprint.groove.swing_ratio`: `Swung` above a swing threshold,
    `Straight` below a straight threshold, `Mixed` between. `None` if no session reported a ratio.
  - `register` ← aggregated session register band (mid by default; see Risks — sourced from the
    fingerprint's register signal where present). `None` when no session carried it.
  - `dynamics` ← aggregated session dynamic level. `None` when no session carried it.
  - `comparison` ← built from the **present** axes, looked up in #253's curated `connections`
    grounding table (mode + feel as the concept key), preferring an exemplar that intersects the
    player's `taste.artists`/`taste.genres` when one exists. **Grounded only** — never fabricated.
    `None` when `confidence` is below the comparison threshold or no curated row matches.
  - `confidence` ← rises with `sessions_counted` and with axis agreement (how consistently each axis
    points the same way). The card shows the comparison only at/above the threshold; otherwise the
    UI renders "still listening…" in the comparison slot while still showing the resolved axes.
- An **absent axis** means "not measured honestly yet" (mirrors the fingerprint contract), never a
  zeroed value — the card simply omits that line.

### Frontend (`apps/desktop/src/components/SoundMirrorCard.tsx`, types in `types/brain.ts`)
- One clean identity card (Tailwind, reuses the recap/`CoachingTipPanel` visual language). Renders
  the resolved axes as one sentence plus the grounded comparison line.
- Below `MIN_SESSIONS` (no `SoundProfile`): the **empty state** — "Keep playing to discover your
  sound" with a count toward K, no guessed traits.
- `SoundProfile` present but `comparison == None`: show the axes and a quiet **"still listening…"**
  where the comparison would go (never a guessed artist).
- TS mirror types added to `types/brain.ts` with a roundtrip assertion (per that file's drift rule).
- No business logic in the component — it renders the snapshot the Rust core derived (CLAUDE.md).

## 5. Acceptance criteria (numbered, testable)
1. Given fewer than `MIN_SESSIONS` fingerprints, `derive_sound_profile` returns `None`; the card
   renders the "keep playing to discover your sound" empty state (no traits, no comparison).
2. Given ≥ `MIN_SESSIONS` fingerprints, `derive_sound_profile` returns `Some(SoundProfile)` whose
   `sessions_counted` equals the number of input fingerprints.
3. Given a set whose present `key`s are predominantly minor-family (≥ the fraction), `mode_lean` is
   `Minor`; predominantly major-family → `Major`; an even split → `Balanced`.
4. Given a set whose median `groove.swing_ratio` is above the swing threshold, `feel` is `Swung`;
   below the straight threshold → `Straight`; in-between → `Mixed`.
5. Calling `derive_sound_profile` twice on the **same** fingerprint slice + taste yields a byte-for-
   byte identical `SoundProfile` except `derived_at` (deterministic aggregation, AC #258.3 of issue).
6. Appending a new session and re-deriving updates the snapshot (e.g. `sessions_counted` increments
   and an axis can move) without panicking and without reordering-sensitivity beyond the defined
   rules (proves "updates as new sessions land").
7. When `confidence` ≥ the comparison threshold and a curated row matches the resolved axes,
   `comparison` is `Some` and its `label` is built from an exemplar that exists in #253's curated
   `connections` table (never fabricated); when the player's `taste` intersects a curated exemplar,
   that exemplar is preferred.
8. When `confidence` is below the comparison threshold, `comparison` is `None` and the card shows
   "still listening…" in the comparison slot while still rendering the resolved axes.
9. An axis whose source dimension is absent in **every** input fingerprint is `None`, and the card
   omits that line rather than printing a default/zero.
10. The `SoundProfile` written to `LearnerModel.sound_profile` round-trips losslessly through F2's
    JSONB blob, and an unknown future field on the blob is preserved across a read/write (forward-
    compatible, per #252 §F2 invariant).
11. No outbound network call originates from this feature: with #253's LLM enrichment opted out, the
    comparison still resolves from curated text and the connections client is never invoked.

## 6. Edge cases & failure modes
- **Cold start / 0 sessions:** `derive_sound_profile` → `None`; empty state with a 0/K counter. No crash.
- **Exactly `MIN_SESSIONS`-1 vs `MIN_SESSIONS`:** boundary tested both sides (off-by-one guard).
- **All fingerprints thin** (every axis dimension absent — e.g. all silence/short sessions): returns
  `Some` only if ≥K sessions, but every axis `None` and `comparison` `None`; card shows the count and
  "still listening…", never invents traits.
- **Conflicting signal** (half minor, half major / swing ratios straddling thresholds): resolves to
  `Balanced` / `Mixed` and **lowers** `confidence` so the comparison gates off — no false certainty.
- **No curated match** for the resolved axis combination: `comparison` `None` even above threshold
  (grounded-only rule from #253 — never fabricate).
- **Empty taste profile:** comparison still resolves from the curated table on axes alone; taste only
  *biases* exemplar choice, it is not required.
- **Reordered / duplicated fingerprints:** aggregation is over the multiset of values per the defined
  rules; a roundtrip/determinism test pins that the same set → same result.
- **Schema drift on the blob:** version field + preserve-unknown-fields (F2), guarded by AC10.
- **Offline / LLM opt-out:** curated path only; no call (AC11).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 / cold start | `brain::learner::tests::below_k_returns_none` | `< MIN_SESSIONS` → `None` |
| AC1 (UI) | `SoundMirrorCard.test.tsx::empty_state` | renders "keep playing", shows count/K, no traits |
| AC2 | `tests::counts_sessions` | `sessions_counted == input.len()` at/above K |
| AC3 | `tests::mode_lean_minor_major_balanced` | three cases map to `Minor`/`Major`/`Balanced` |
| AC4 | `tests::feel_swung_straight_mixed` | swing-ratio median → `Swung`/`Straight`/`Mixed` |
| AC5 | `tests::derive_is_deterministic` | same slice+taste → identical profile (ignoring `derived_at`) |
| AC6 | `tests::rederive_after_new_session_updates` | append → `sessions_counted++`, no panic |
| AC7 | `tests::comparison_is_curated_and_taste_preferred` | exemplar ∈ #253 table; taste intersection preferred |
| AC8 | `tests::low_confidence_gates_comparison` | below threshold → `comparison` `None` |
| AC8 (UI) | `SoundMirrorCard.test.tsx::still_listening` | axes shown, "still listening…" in comparison slot |
| AC9 | `tests::absent_axis_is_none` | dimension absent in all → axis `None` (UI omits line) |
| AC10 | `tests::sound_profile_roundtrip_preserves_unknown` | lossless blob roundtrip + unknown field kept |
| AC11 | `connections::tests::no_network_for_sound_mirror` | curated comparison, client never invoked |
| boundary | `tests::k_minus_one_vs_k` | `None` at K-1, `Some` at K |
| conflict | `tests::conflicting_signal_lowers_confidence` | split signal → `Balanced`/`Mixed` + gated comparison |
| Manual | identity card in a long-lived profile | one clean card, correct empty / still-listening / full states |

## 8. Architecture / approach
`derive_sound_profile` lives in `brain::learner` alongside the other F2 transitions and reads the
already-stored `MusicalFingerprint`s (`sessions.fingerprint`, migration 0004) plus the local
`TasteProfile` — **no new perception, no audio-thread work, no new IPC philosophy**. The grounded
comparison **reuses #253's curated `connections` table** (`brain::connections`) so there is one
source of musical truth and one opt-in/disclosure surface; no second grounding dataset and no
feature-specific network path (offline-first per CLAUDE.md). The result is stored on
`LearnerModel.sound_profile` and travels through F2's existing local store + Supabase blob sync
(nullable, additive — no row migration). A thin command surfaces the snapshot; the frontend
`SoundMirrorCard` is pure read-model rendering. All aggregation, gating, and grounding stay in the
Rust core; the component never computes a trait.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | `SoundProfile` types + `derive_sound_profile` core: K-gate, `mode_lean` + `feel` axes, determinism, F2 roundtrip — comparison `None` for now | `crates/brain/src/learner*` (sound_profile module), F2 blob | F2 | no |
| S2 | `register` + `dynamics` axes + `confidence` aggregation (axis agreement) | `crates/brain/src/learner*` (sound_profile) | S1 | no |
| S3 | Grounded `comparison` via #253 curated table (axes→concept, taste-preferred exemplar, confidence gate, opt-out safe) | `brain::learner` sound_profile + `brain::connections` (read) | S1, S2, #253 | no |
| S4 | IPC command + `SoundMirrorCard` + TS mirror types/roundtrip: empty / still-listening / full states | `apps/desktop/src-tauri/src/commands.rs`, `apps/desktop/src/components/SoundMirrorCard.tsx`, `apps/desktop/src/types/brain.ts`, `App.tsx` wiring | S1–S3 | no |

S1 is the seam (types + deterministic core) others render and ground behind. S1/S2 share the
sound_profile module (sequential); S3 adds the connections read; S4 is the disjoint frontend slice.

## 10. Risks / open questions
- **Register & dynamics sourcing:** the current `MusicalFingerprint` (0004) exposes `tone`, `key`,
  `intonation`, `groove` but no direct session-level *register band* or *dynamic level* aggregate.
  Open question for S2: derive these from an existing dimension (e.g. register from the pitch
  signal already summarised per phrase, dynamics from a session dynamics aggregate) **or** add a
  small additive, non-audio-thread session-level field computed during the existing recap
  aggregation. Either way it must stay off the hot path and absent-when-unmeasured. Until resolved,
  S1 ships with `register`/`dynamics` always `None` (axes simply omitted) so the slice still lands.
- **Threshold tuning** (K, minor-family fraction, swing/straight cutoffs, comparison confidence gate)
  needs a few real multi-session profiles to feel right — they are named constants, easy to tune.
- **Curated coverage:** #253's table must cover the common axis combinations or the comparison gates
  off often. Start with mode×feel combos that have clear exemplars; expand with #253. Grounded-only
  is the rule — a missing row means "still listening…", never a guess.
- **Comparison phrasing** ("like a young X") for a kids' tool must read as inspiring, never as a
  ceiling/label — copy reviewed with the same "coach, don't judge" bar as `TasteOnboarding`.

## 11. References
- Epic spec `docs/specs/252-rv-practice-coach.md` (§F2 Learner Model: `derive_sound_profile`,
  `sound_profile` field, blob invariants). Style + curated grounding table: `docs/specs/253-reveal-loop.md`.
- Existing data: `supabase/migrations/0004_personalization_fingerprint_and_taste_profile.sql`
  (`sessions.fingerprint`, `taste_profile`), `apps/desktop/src/types/brain.ts`
  (`MusicalFingerprint`, `KeyEstimate`/`Mode`, `GrooveDescriptor`, `TasteProfile`),
  `apps/desktop/src/stores/tasteStore.ts`, `apps/desktop/src/components/TasteOnboarding.tsx`.
- Issue #258; epic #252; #253 (curated `connections` grounding reused for the comparison).
