# Platform Spine — Personalization / Cross-Genre Context

**Companion doc to:** [`architecture-v2.md`](./architecture-v2.md) and [`platform-modules-addendum.md`](./platform-modules-addendum.md)
**Status:** Draft
**Date:** 2026-06-07

> **Forward references:** the Phase 4 musical-relevance feature (`MusicalFingerprint`, the relevance engine, the grounding/anti-hallucination contract) and the teacher-dashboard privacy story are not yet written. This spine treats them as *planned* docs; section numbers below are placeholders to be reconciled when those docs land. The existing privacy/audit precedent is [`teacher-audit.md`](./teacher-audit.md). See the Open questions at the end.

---

## Purpose

This is the **Personalization / Cross-Genre spine** — one of the platform's **three shared spines**. The other two are specified separately and referenced here by name: the **Commerce / Entitlements spine** ([`platform-spine-commerce.md`](./platform-spine-commerce.md)) and the **Unified Content Format spine** ([`platform-spine-content-format.md`](./platform-spine-content-format.md)). This spine is **one student musical profile, read by many consumers**: a single SQLite-backed record of stated preferences (genres, artists, goals, experience) that enriches every coaching output, the AI Listening Coach, and content tailoring. It is the connective tissue that makes a Bach etude feel relevant to a kid who loves Kendrick — without each module inventing its own profile.

---

## Layman's overview

> The app asks you a few questions once — what you listen to, who you love, why you're here — and remembers. From then on, *everything* the coach says is tuned to you: the tips, the recaps, the "this is the same pocket as a D'Angelo record" connections. There is one "about you" file, and every part of the app reads from it. Nobody builds a second one, and we never sell it.

---

## The profile data model (SQLite)

The profile lives in the existing local SQLite store (`rusqlite`, alongside `sessions` / `session_phrases`) and, when the user opts into sync, in the Supabase `profiles` neighbourhood already established by the teacher-dashboard track (#144). Business logic for assembling and enriching prompts stays in the Rust core; IPC carries thin JSON.

```
taste_profile (1 row per local user, joins to profiles.id when synced)
  user_id           TEXT  -- FK to profiles.id (local-first; synced if sync on)
  genres            JSON  -- ["hip-hop","film score","gospel"]  (stated)
  artists           JSON  -- ["Kendrick Lamar","Hans Zimmer"]    (stated)
  goals             JSON  -- ["audition prep","play in church band"] (stated)
  experience        TEXT  -- enum: beginner | intermediate | advanced
  derived_signals   JSON  -- learned: which connections landed (thumbs, replays)
  updated_at        TEXT
```

- **Stated, editable, evolving.** Captured at onboarding (a quick "pick a few artists / genres / why you're here"), editable any time, and refined by `derived_signals` — lightweight feedback about which cross-genre connections the student engaged with. This is the same taste profile described in story-phase4 §3.2 (ownership boundary in the Reconciliation table below).

### Relationship to the Phase 4 `MusicalFingerprint`

These are **distinct and must stay distinct**, but they join at coaching time:

| | `MusicalFingerprint` (story-phase4 §3.1) | `taste_profile` (this spine) |
|---|---|---|
| Nature | **Performance facts** — measured this take | **Stated preferences** — who the student is |
| Source | DSP in `crates/theory` / `crates/tone`, confidence-tagged | Onboarding + edits + derived feedback |
| Lifetime | Per phrase / per session | Persistent, slowly evolving |
| Truth status | Ground truth the LLM may *assert* | Context the LLM may *use to frame*, never asserts as a fact |

The relevance engine joins them: the fingerprint says *what was played* (D dorian, laid-back groove); the profile says *whose world to connect it to* (D'Angelo, not just "So What"). Keeping them separate is what protects grounding — preferences never become claimed performance facts.

---

## The consumers — one profile, many readers

The profile is written once and read by every coaching surface. No consumer owns its own copy.

1. **Every coaching tip / recap prompt (Brain, Phase 1).** The LLM Coach's existing per-instrument system prompts (architecture-v2 §3b) gain a small profile-derived block: experience level shapes vocabulary and depth; goals shape emphasis; genres/artists are available for framing. This is the cross-genre prompt enrichment named in the addendum's "Cross-Genre Contextual Coaching" row — a few lines injected into the existing prompt, not a new engine.
2. **The AI Listening Coach references (Module 3).** When the engine selects a reference to illustrate a concept, the profile biases *which* real example it reaches for, so connections land in the student's world. The references themselves and their grounding are owned by story-phase4 (see below) and the shared **Unified Content Format**; this spine only supplies the *taste signal* that ranks them.
3. **Content-pack tailoring.** Audition packs, curricula, and knowledge entries can be ordered/surfaced by relevance to goals and experience. The packaging, entitlement, and format of that content belong to the **Unified Content Format** and **Commerce / Entitlements** spines — this spine contributes only the read-side taste signal. See those docs; do not duplicate their schemas here.
4. **Future modules.** Any new module that wants "who is this student" reads the same row. The contract is: the profile is read-only to consumers, mutated only through the onboarding/edit flow and the derived-signal feedback path.

---

## Grounding / anti-hallucination

The personalization spine **adds taste, not facts**, and that is precisely how it stays safe. It does not introduce any new vector for fabricated artist/track claims:

- Cross-genre references remain grounded exactly as specified in story-phase4 §3.4: the LLM may assert performance facts **only** from the confidence-tagged `MusicalFingerprint`; cultural/artist claims are web-search-backed and must resolve, degrading to genre-level or silence when unverifiable.
- The profile feeds the *framing and selection* layer ("connect this to their world"), never the *assertion* layer. A student's stated love of an artist does not let the model claim that artist plays a given lick — that claim still has to come from grounded retrieval or the symbolic idiom matcher (story-phase4 §3B).
- Confidence-gating is unchanged: low feature confidence → hedged voice, regardless of how rich the profile is. We do not let a vivid taste profile tempt the model into false precision.

---

## Privacy

Reconciled with the existing privacy/audit precedent ([`teacher-audit.md`](./teacher-audit.md)) and the planned teacher-dashboard privacy story (not yet written — see forward-references note above):

- **What's stored:** preference data only — genres, artists, goals, coarse experience level, and engagement-derived signals. No birthdate: we reuse the **existing** `is_under_13` flag shipped by [`teacher-audit.md`](./teacher-audit.md), never a DOB. (If teen-consent rules later need finer granularity, `is_under_13` generalizes to a coarse `age_tier` enum platform-wide — one primitive, evolved, not a second one.) The profile is *preference* data, categorically less sensitive than a minor's audio or PII.
- **Local-first.** The profile lives on-device by default and only reaches Supabase if the user turns on sync — the same independent-switches model as session data (#145). Linking and syncing remain separate from this profile entirely.
- **Minors.** For under-13, keep the profile minimal and parent-visible, reusing the existing consent component rather than building a second one (story-phase4 §3.2; privacy doc §3). The taste profile is **not** part of what crosses to a teacher: the data-visibility matrix (privacy doc §4) governs teacher sharing, and preference data is not in the shared set unless a future, separately-consented story adds it.
- **Erasure.** Deleting the profile is covered by the existing "delete my cloud data" action; it cascades like other owned rows.

---

## Reconciliation with story-phase4-musical-relevance.md

To avoid duplication, the boundary is explicit:

| Concern | Owned by **story-phase4** (the feature) | Owned by **this spine** (the platform layer) |
|---|---|---|
| `MusicalFingerprint` (measured facts) | ✅ defines & produces it | references it; keeps it distinct from profile |
| Taste profile *consumption* by the relevance engine | ✅ relevance generation, connections, "What was that?" | — |
| Taste profile *schema, lifecycle, storage, privacy* | uses it | ✅ owns it as a platform record |
| Grounding / anti-hallucination contract | ✅ defines it | re-uses it, adds no new claim vector |
| Idiom recognition (symbolic + retrieval) | ✅ entirely | — |
| Making the profile readable by *all* consumers (tips, recaps, Listening Coach, content packs, future modules) | — | ✅ the "one profile, many consumers" contract |

---

## Phased delivery

Mapped to the addendum roadmap:

- **Phase 1 (small add).** Ship the `taste_profile` table + onboarding form + the few-line prompt-enrichment block in the existing LLM Coach. This is the "student musical profile" the addendum flags as the one worthwhile Phase 1 addition — optional, late-Phase-1 or early-Phase-2 story. The system prompts can begin referencing genres immediately, with no reference DB required.
- **Phase 2.** Content-pack tailoring reads the profile once the Unified Content Format / Commerce spines land packs and entitlements.
- **Phase 3+.** AI Listening Coach (Module 3) and the Phase 4 relevance/idiom work consume the same profile and add `derived_signals` refinement. The profile schema does not change shape across phases — consumers are added, the record is not rebuilt.

---

## What we are deliberately NOT building

- **No behavioral ad targeting.** The profile exists to coach, full stop.
- **No selling or sharing preference data.** It is never a product, never brokered, never sent to third parties beyond the user's own optional sync.
- **No recommendation engine beyond coaching context.** We do not build a "songs you might like" / discovery feed. Taste ranks coaching references; it does not become a media recommender.
- **No second profile.** Modules do not fork their own preference tables. One row, many readers — enforced by code review behind this spine.
- **No inferred sensitive attributes.** We store what the student tells us and what they engage with; we do not infer demographics, mood, or anything beyond musical taste signals.
- **No profile on the real-time path.** Personalization is reflective (prompts, recaps, on-demand) — never on the <25 ms mic-to-screen path.

---

## Open questions

1. **Onboarding depth** — quick "pick 5 artists" vs a richer survey, and how much we infer from `derived_signals` vs ask outright (mirrors story-phase4 Q3; this spine inherits the answer).
2. **Derived-signal feedback loop** — what exactly counts as "a connection landed" (replay, thumbs, time-on-card), and where that feedback is captured without nagging the user.
3. **Profile portability across modules vs. namespacing** — do all modules read the whole profile, or do we expose scoped views (e.g. a content-pack reader sees genres + goals but not raw artist list)?
4. **Does the profile ever cross to a teacher?** Default is no (not in the privacy doc's shared set). If a future story wants teachers to tailor assignments by taste, it needs its own consent gate — flagged here, not decided.
5. **Cold-start framing** — how the coach behaves before any profile exists (proposal: instrument-default prompts, genre-neutral, exactly as today).
