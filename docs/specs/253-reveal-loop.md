# Spec: Reveal loop — ambient real-world music connections (#253)

> Part of epic #252. The first feature to build: the smallest slice that proves the magic, and it
> depends only on **existing perception output**, not on the F1/F2 foundations.

## 1. Summary
While a player free-plays, the AI occasionally surfaces a **reveal**: the real-world music that lives
in what they just played — a scale/mode → an artist, a famous piece, a genre, a line of history.
Reveals are unobtrusive, accurate, and (later slice) collectible.

## 2. Problem / why
Free play hears the player (perception already emits a live key/mode estimate — see `PerceptionPanel`)
but does nothing with it. The recap's "Flavour"/"In your world" cultural connection is buried on an
end screen. This promotes that idea to a constant, delightful, *educational* reward during play — the
dopamine core of the epic and the user's explicit "relate everything to the real world of music" goal.

## 3. Non-goals (this issue)
- No collection persistence / Learner Model write **in slice 1** (that's slice 2, on F2).
- No "play the exemplar" audio or chips (later slice).
- No new pitch/scale detection — reuse the existing perception key/mode estimate.
- Not more than one reveal on screen at a time.

## 4. Contract / interface
Backend (Rust, `brain::connections`):
```rust
pub struct MusicalContext {           // built from EXISTING perception output
    pub key: String,                  // e.g. "G"
    pub mode: String,                 // e.g. "dorian" / "major" / "minor"
    pub confidence: f32,              // 0..1, from perception
}
pub struct Reveal {
    pub concept: String,              // "G Dorian"
    pub connection: String,           // "Santana — Oye Como Va"
    pub why: String,                  // one line, ≤140 chars
    pub source: RevealSource,         // Grounded | LlmGrounded  (never ungrounded)
}
/// Pure selection over the curated table; deterministic for a given context + picker seed.
pub fn reveal_for(ctx: &MusicalContext, seed: u64) -> Option<Reveal>;
```
- A curated `connections` table (concept → exemplars) ships in-crate as the **grounding** (data file).
  When the LLM path is enabled, it may *rephrase/enrich* `why` but **must not invent** the connection —
  the artist/piece always comes from the curated table. `reveal_for` returns `None` below a confidence
  threshold or when the context has no curated match.
- New IPC event `reveal` (payload `Reveal`), emitted at most once per **N** phrases (default 3).
- Opt-in: LLM enrichment is gated by the existing coaching opt-in; with it off, reveals still work
  using curated `why` text and make **no** network call.

Frontend (`apps/desktop/src/components/RevealCard.tsx`):
- Subscribes to `reveal`; renders one card (concept, connection, why) reusing `CoachingTipPanel`
  styling; auto-dismiss after ~12s or on the next reveal; never stacks.

## 5. Acceptance criteria (numbered, testable)
**Slice 1 (this build):**
1. Given a `MusicalContext` with confidence ≥ threshold and a curated match, `reveal_for` returns a
   `Reveal` whose `connection` is exactly one of the curated exemplars for that concept.
2. Given confidence **below** threshold, `reveal_for` returns `None` (no guess).
3. Given a context with **no** curated match, `reveal_for` returns `None` (never fabricates).
4. The `reveal` event is emitted at most once per N phrases — given 5 rapid phrases with N=3, exactly
   ≤2 reveals are emitted.
5. With LLM enrichment opted out, a reveal is still produced from curated text and **no outbound
   network call** is made (assert the connections client is never invoked).
6. `reveal_for` is deterministic for a fixed `(ctx, seed)` (same exemplar chosen).
7. Frontend: on a `reveal` event a single `RevealCard` renders with concept+connection+why; a second
   event replaces (does not stack) it.

## 6. Edge cases & failure modes
- Perception emits a key but unknown/ambiguous mode → treat as no curated match → `None`.
- Rapid-fire phrases → rate limiter holds to ≤1 per N (idempotent; tested).
- LLM enabled but call fails/times out → fall back to curated `why`, still show the card (never block).
- Empty/garbage context (silence) → no reveal.
- Offline → curated path only; no call.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `brain::connections::tests::returns_curated_exemplar` | connection ∈ curated set for concept |
| AC2 | `tests::low_confidence_returns_none` | None below threshold |
| AC3 | `tests::no_match_returns_none` | never fabricates |
| AC4 | `tests::rate_limited_per_n_phrases` | ≤1 per N over a phrase burst |
| AC5 | `connections::tests::no_network_when_opted_out` | client not invoked offline/opt-out |
| AC6 | `tests::reveal_is_deterministic` | same (ctx,seed) → same exemplar |
| AC7 | `RevealCard.test.tsx` | renders fields; second event replaces |
| LLM fail | `tests::llm_failure_falls_back_to_curated` | card still produced |

## 8. Architecture / approach
`MusicalContext` is assembled from the existing perception event (`PerceptionPanel` already consumes
key+confidence; extend with the mode estimate already computed in brain). Selection logic + the
curated table live in Rust core (`brain::connections`); the frontend only renders. LLM enrichment
reuses the existing coaching client and its opt-in; it is disclosed in `ConnectionsPrivacy.tsx` + the
network allowlist (offline-first). Rate limiting lives with the phrase pipeline that already exists
for coaching tips, so we don't add a second cadence source.

## 9. Slice breakdown (ordered, shippable PRs)
| # | Slice (goal) | Footprint | Depends on | Heavy |
|---|---|---|---|---|
| S1 | `reveal_for` + curated table + `reveal` event + rate limit + `RevealCard` (curated-only, opt-out safe) | `crates/brain/src/connections*`, `apps/desktop/src-tauri/src/commands.rs`, `apps/desktop/src/components/RevealCard.tsx`, `App.tsx` wiring | existing perception | yes |
| S2 | LLM enrichment of `why` (opt-in, grounded, graceful fallback) + disclosure | `brain::connections` llm, `ConnectionsPrivacy.tsx`, allowlist | S1 | no |
| S3 | Persist to Learner Model collection (dedup + count) + tiny collection count UI | `brain::learner` (F2), `RevealCard`/collection UI | S1, F2 | no |

**First slice to build now: S1.** It is self-contained, offline-safe, and proves the loop.

## 10. Risks / open questions
- Curated table scope: start small (church modes + a dozen common scales → 2–3 exemplars each); expand
  later. Accuracy bar is high (kids) — grounded-only is the rule.
- Mode estimate availability in brain: if not already exposed over IPC, S1 adds the field to the
  perception payload (small, additive).

## 11. References
- `apps/desktop/src/components/PerceptionPanel.tsx` (existing key/mode estimate), `CoachingTipPanel.tsx`
  (card styling + cadence), `crates/brain` (phrase pipeline), epic #252, RV `src/data/scales.json`.
