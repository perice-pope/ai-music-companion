# Spec: RV-powered AI practice coach (Epic #252)

> Umbrella spec. Owns the two shared **foundations** (RV generator engine, Learner Model) and the
> wave plan for the seven feature slices (#253–#259). Each feature has its own spec
> (`docs/specs/<n>-*.md`); this one defines the seams they build behind.

## 1. Summary
Turn free play into a single, addictive practice space: the AI listens and either ambiently
suggests Random-Variations (RV) material to explore or, on demand, runs an adaptive guided lesson —
always tying what you play back to real-world music. One tool fits beginner→advanced because RV is
simple-or-complex by design. Every session compounds into an evolving per-user **Learner Model**.

## 2. Problem / why
Today free play hears you but offers nothing to *do* — no suggestions, no structure, no progression,
no memory across sessions. RV (`perice-pope/random-variations`) has the practice methodology but is
blind (can't hear you) and stateless. AMC has ears + an LLM coach but no practice content engine and
no learner state. Joining them is the product; the missing pieces are a generator and a memory.

## 3. Non-goals
- No text/voice input from the user (interaction = play + tap AI-offered chips). 
- No social/multiplayer, no teacher-assignment integration in this epic.
- No new instruments or audio-thread changes; we consume existing ears/scoring outputs.

## 4. Contract / interface (the two foundations)

### F1 — RV generator engine — `crates/variations` (new Rust crate)
Pure, deterministic music-theory generation. **No I/O, no randomness except via an explicit seed**
(so every output is reproducible and unit-testable).

```rust
pub struct VariationSpec {
    pub roots: Vec<Pitch>,                 // e.g. the 12 chromatic roots, or a chosen subset
    pub scale: Option<ScaleModifier>,      // scaleType + pattern (up/down/up-down/skip-1/…)
    pub chord: Option<ChordModifier>,      // chordType, broken|stacked, inversion, arpeggio pattern
    pub interval: Option<IntervalModifier>,// 1P..8P, broken|stacked, asc|desc
    pub enclosure: Option<EnclosureModifier>, // approach-note pattern (one-up, two-down, …)
    pub direction: DirectionModifier,      // forward | reversed | random-per-note
    pub rhythm: RhythmSpec,                // beats/divisions, rests/offset between groups
    pub randomize_roots: bool,             // RV's signature: shuffle root order (keep first fixed)
    pub transpose: Transposing,            // C/Bb/A/G/F/Eb instrument view
}

pub struct GeneratedNote { pub midi: u8, pub start_beat: f64, pub duration_beats: f64 }
pub struct GeneratedSequence {
    pub notes: Vec<GeneratedNote>, // renderer-agnostic beat grid; brain::coach adapts to
                                   // ScoreModel → MusicXML (the shape ScoreView consumes)
    pub target_midi: Vec<u8>,      // the exact grading target (what the user should play)
    pub label: String,             // human label e.g. "G Dorian · up-down · 12 roots · 80 BPM"
    pub tempo_bpm: f64,
    pub beats_per_measure: u8,
}

pub fn generate(spec: &VariationSpec, seed: u64) -> GeneratedSequence; // deterministic

// Shipped-contract notes (F1 slice, documented drift from the sketch above):
// instrument `transpose` (C/Bb/A/G/F/Eb) is DEFERRED to the #254 notation
// adapter, where spelling/transposition belong; `stacked` chord/interval
// rendering is deferred (needs chord rendering in the score path); random
// direction is per-ROOT (the RV behavior the coach wants), not per-note.
```
Catalog data (`scales`, `chords`, `enclosures`) is ported from RV's `src/data/*.json` into the crate.
Theory uses an existing Rust crate (e.g. an established music-theory lib) or a thin port of the
needed `tonal` operations — chosen in the F1 slice, not here.

### F2 — Learner Model — `brain::learner` (Rust core) + persistence
A versioned, additive aggregate that every feature reads/writes. Stored as **one JSONB blob** (the
repo already prefers blobs for `sessions.fingerprint`, see migration 0004), persisted locally
(`crates/brain` store) and synced via a new nullable `profiles.learner_model jsonb` column.

```rust
pub struct LearnerModel {
    pub version: u8,
    pub difficulty: u8,                              // 0..=MAX adaptive step
    pub streak: Streak,                              // count, last_completed_local_day
    pub key_mastery: BTreeMap<KeyScale, Mastery>,    // accuracy EWMA + attempts per key/scale
    pub collection: BTreeMap<ConceptKey, Collected>, // unlocked reveals (deduped)
    pub sound_profile: Option<SoundProfile>,         // derived identity ("your sound")
    pub updated_at: Timestamp,
}
pub struct Mastery { pub attempts: u32, pub accuracy_ewma: f32, pub owned: bool, pub last: Timestamp }
pub struct Collected { pub concept: String, pub connection: String, pub first_seen: Timestamp, pub count: u32 }

// Pure, deterministic transitions — the heart of "gets smarter". No I/O.
pub fn apply_drill_result(m: &LearnerModel, r: &DrillResult) -> LearnerModel;
pub fn apply_reveal(m: &LearnerModel, concept: &str, connection: &str, now: Timestamp) -> LearnerModel;
pub fn apply_daily_completion(m: &LearnerModel, day: LocalDay, score: f32) -> LearnerModel;
pub fn derive_sound_profile(sessions: &[Fingerprint], taste: &TasteProfile) -> Option<SoundProfile>;
```
**Invariants:** transitions are pure + deterministic (same input → same output); the blob is
forward-compatible (unknown fields preserved on read); accuracy via EWMA so it adapts but is stable;
mastery `owned` flips only at a defined threshold over ≥M attempts.

### Interaction contract (frontend)
The AI's "voice" is a single `CoachCard { reaction?, reveal?, chips: Chip[] }` rendered in free play.
`Chip` = `{ label, action: VariationDelta | LessonStart | RevealExpand }`. The Explore→Coach dial is
one control that raises structure (chips → guided routine). No new IPC philosophy — extends the
existing event/command pattern.

## 5. Acceptance criteria (epic-level, end-to-end)
1. From cold free play, playing a clear modal phrase yields at most one reveal card and persists one
   collection entry (proves F2 + #253 wired).
2. Tapping "Give me a lesson" produces an RV routine whose drills are scored against `target_notes`
   and whose results change `difficulty` and `key_mastery` in the Learner Model (proves F1+F2+#254).
3. The Learner Model round-trips losslessly local↔Supabase and is forward-compatible across a version
   bump (unknown fields preserved).
4. With reveals/coaching opted out, no outbound network call is made anywhere in the epic.

## 6. Edge cases & failure modes
- First run: empty Learner Model → every feature renders a defined empty state (no crash, no guess).
- Low detection confidence → no reveal/suggestion rather than a wrong one.
- Offline → generator + scoring + mastery all work; only LLM reveals/coaching are gated off.
- Schema drift → version field + preserve-unknown-fields; a roundtrip test guards it.
- Determinism → generator and all `apply_*` transitions must be seed/Input-deterministic (tested).

## 7. Test plan
| AC / invariant | Test | Asserts |
|---|---|---|
| F1 deterministic | `variations::tests::generate_is_seed_deterministic` | same spec+seed → identical sequence |
| F1 randomize keeps first | `variations::tests::shuffle_keeps_first_root` | RV rule honored |
| F2 EWMA + owned flip | `brain::learner::tests::owned_flips_at_threshold` | mastery transition rule |
| F2 forward-compat | `brain::learner::tests::roundtrip_preserves_unknown` | blob versioning |
| AC1 | e2e `reveal_persists_one_collection_entry` | reveal→collection +1 |
| AC2 | integration `lesson_updates_difficulty_and_mastery` | adaptive loop |
| AC4 | `connections::tests::no_call_when_opted_out` | offline-first |

## 8. Architecture / approach
F1 is a leaf crate (no deps on brain) so anything can call it. F2 lives in `brain` next to the
existing session/fingerprint code and reuses the SQLite store + Supabase sync; the new column is
nullable + additive (no migration of existing rows). Reveals/coaching reuse the existing LLM path and
must be opt-in + disclosed in `ConnectionsPrivacy.tsx` + the network allowlist. Frontend is read-model
+ chips; **all generation, scoring, and learner transitions stay in Rust core** per CLAUDE.md.

## 9. Slice breakdown (waves)
| # | Slice (goal) | Footprint | Depends on | Heavy |
|---|---|---|---|---|
| F1 | RV generator engine + catalog + seed determinism | `crates/variations/**` | — | yes |
| F2 | Learner Model struct + pure transitions + store + `profiles.learner_model` column | `crates/brain/src/learner*`, `supabase/migrations/0005_*` | — | yes |
| #253 | Reveal loop (uses F2 collection + LLM grounding) | `brain::connections`, `apps/desktop/src/components/Reveal*` | F2(min) | yes |
| #255 | Suggester chips + RV shuffle | `apps/desktop/src/components/CoachCard*`, commands | F1 | no |
| #256 | 12-key mastery wheel (read view) | `apps/desktop/src/components/KeyWheel*` | F2 | no |
| #254 | Adaptive guided coach | `brain::coach`, ScoreView reuse | F1,F2 | yes |
| #257 | Streak + daily roulette | `brain::learner` streak, `apps/desktop/src/...` | F1,F2 | no |
| #258 | "Your sound" mirror | `brain::learner::sound_profile`, UI card | F2 | no |
| #259 | Boss moments + band | `brain::coach` moments, accompaniment | F1,F2,#212 | yes |

Wave 0: F1, F2 (disjoint, alone). Wave 1: #253, #255, #256. Wave 2: #254, #257. Wave 3: #258, #259.

## 10. Risks / open questions
- Music-theory lib choice for F1 (port vs dependency) — decide in F1 slice.
- Reveal accuracy: how much curated grounding vs LLM (kids' tool — bias to grounded). Tracked in #253.
- Where the Coach dial lives in the existing free-play layout — resolved by the UX wireframe (#256/#253 UI).

## 11. References
- RV source: `perice-pope/random-variations` (`src/musicUtils.ts`, `src/types.ts`, `src/data/*.json`).
- Existing: migration `0004_personalization_fingerprint_and_taste_profile.sql`, `crates/brain/src/store.rs`, `apps/desktop/src/types/brain.ts`, `CoachingTipPanel.tsx`, `ScoreView.tsx`, accompaniment #212.
