# Platform Spines — How the Platform Stays One System

**Companion doc to:** [`architecture-v2.md`](./architecture-v2.md) and [`platform-modules-addendum.md`](./platform-modules-addendum.md)
**Status:** Draft
**Date:** 2026-06-07

---

## The one idea

The Platform Strategy lists **five revenue modules** (Audition Simulator, Instrument
Knowledge Engine, AI Listening Coach, Teacher Curriculum Marketplace, B2B Licensing).
Built as five features, that becomes five storefronts, five content formats, five
notions of "who is this student" — five things to keep in sync forever.

We don't build five modules. We build **three shared spines**, and every module is a
thin arrangement of the three. Simplicity is mastery: one way to sell, one way to
package content, one way to know the student.

| Spine | One sentence | Doc |
|---|---|---|
| **Commerce / Entitlements** | One entitlements model grants access to *anything* — packs, curricula, B2B seats. | [`platform-spine-commerce.md`](./platform-spine-commerce.md) |
| **Unified Content Format** | One package format where an audition pack is the 1-section case and a curriculum the multi-week case. | [`platform-spine-content-format.md`](./platform-spine-content-format.md) |
| **Personalization / Cross-Genre** | One student musical profile, read by every coaching surface. | [`platform-spine-personalization.md`](./platform-spine-personalization.md) |

---

## Modules are arrangements of spines

Read this table left-to-right: a module is "what it sells × what it packages × how it
personalizes." No module owns infrastructure; it composes the spines.

| Module (from the addendum) | Commerce | Content Format | Personalization |
|---|---|---|---|
| 1 · Audition Simulator | sells packs (one-off) | a pack = 1 section | tunes coaching to the student |
| 2 · Instrument Knowledge Engine | (free / bundled) | knowledge entries reference packs | surfaces tips by what they play |
| 3 · AI Listening Coach | premium tier | references attach to steps | drives the cross-genre examples |
| 4 · Teacher Curriculum Marketplace | sells curricula (70/30 split) | a curriculum = N sections | personalizes pacing |
| 5 · B2B Licensing | sells seats | partners ship packs | white-label profile context |

If a proposed feature needs a *fourth* spine, that is the signal to stop and question
the feature — not to add the spine.

---

## How this connects to what we're building now

The spines are not greenfield. The current Phase 3/4 work already lays their
foundations, which is the point — the companion and the platform are the same system:

- **Personalization** sits directly on the Phase 4 relevance work. The measured
  `MusicalFingerprint` (tone, key, intonation, groove) and the *stated* taste profile
  are kept distinct and joined only at coaching time. See
  [`story-phase4-musical-relevance.md`](../design/story-phase4-musical-relevance.md).
- **Commerce** reuses the Supabase + RLS posture already shipped (dark) for teacher
  linking — the same self-read / `exists(...)` join idioms, the same service-role-only
  write rule.
- **Content Format** reuses Score Mode, MusicXML import, and the LLM Coach rubric
  prompts verbatim; the only new code is a loader in `crates/brain`.

---

## What we are deliberately NOT doing

- No per-module storefront, content format, or profile. One of each, shared.
- No new spine without a module that genuinely cannot be expressed in the existing three.
- No business logic in the frontend — the spines live in the Rust core; IPC stays thin JSON.

---

## Open cross-spine questions

These are owned at the platform level, not by any single spine doc:

1. **`content_ref` contract.** Commerce entitlements point at Content Format packs by
   stable `pack_id`. The exact shape of that reference is the one hard seam between the
   two spines and should be pinned before either ships.
2. **Branch reconciliation.** These docs currently live on a feature branch behind
   `main`; they cross-reference `story-phase4-musical-relevance.md`, which is on `main`.
   The references resolve once the branch is reconciled with `main`.
