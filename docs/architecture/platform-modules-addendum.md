# Platform Modules Addendum — Mapping the Platform Strategy onto the v2 Architecture

**Companion doc to:** [`architecture-v2.md`](./architecture-v2.md) and `AI_Music_Companion_Platform_Strategy_v3.docx` (lives in the public pitch repo under [`drafts/`](https://github.com/perice-pope/ai-music-companion-pitch/blob/main/drafts/AI_Music_Companion_Platform_Strategy_v3.docx))
**Status:** Approved addendum (does not supersede v2 — extends it)
**Date:** April 19, 2026

---

## Purpose of this document

The Platform Strategy v3 describes **five expansion modules** and a **cross-genre coaching philosophy** that transform the practice companion into a multi-revenue-stream platform. This addendum answers one question: **what changes in the technical architecture?**

Short answer: very little. The v2 architecture was designed with the right seams. The platform modules plug into existing layers without restructuring the core.

---

## Layman's overview

> The v2 architecture has three layers: Ears (listening), Brain (thinking), Face (showing). The platform strategy adds five business modules on top of this.
>
> Think of it like a smartphone. The Ears/Brain/Face are the phone's hardware and OS — they don't change when you install new apps. The five modules are the apps. Each one plugs into capabilities that already exist:
>
> - **Audition Simulator** = a specialized playlist in Score Mode with seasonal content packs
> - **Instrument Knowledge Engine** = a reference library the AI coach reads from
> - **Listening Coach** = an audio player connected to coaching tips
> - **Teacher Marketplace** = a storefront on the existing Teacher Dashboard
> - **B2B Licensing** = packaging the Rust core as a library for partners
>
> None of these require rebuilding the foundation. That's the whole point.

---

## Module-by-module architecture mapping

### Module 1: Audition Simulator

**What it is:** State-specific etude packages with AI coaching tuned to audition criteria (tone, technique, musicality, sight-reading).

**Architecture impact:**

| Layer | Change | Scope |
|---|---|---|
| Ears | None | — |
| Brain | Add `audition_rubric` field to coaching prompts. The LLM coach already exists; this is a system prompt variant that weights audition-specific criteria. | Prompt engineering + 1 new JSON schema |
| Brain | Add content packaging system: bundles of MusicXML + metadata (state, year, instrument, rubric) stored as structured packages. | New `content-packs/` directory + loader module |
| Face | Add "Audition Prep" mode selector in the UI. Reuses Score Mode with a rubric overlay panel. | 1 new React component + route |
| Cloud | Stripe integration for seasonal purchases ($29–49 per pack). Supabase table for entitlements. | Standard e-commerce pattern |

**Phase 1 impact:** None. This is Phase 2 work at earliest.

---

### Module 2: Instrument Knowledge Engine

**What it is:** A curated knowledge base of instrument-specific secrets (embouchure, alternate fingerings, reed adjustment, bow pressure, mouthpiece buzzing) that the AI coach references in real time.

**Architecture impact:**

| Layer | Change | Scope |
|---|---|---|
| Ears | None | — |
| Brain | Add a structured knowledge base (markdown/JSON files per instrument family) that gets injected into coaching prompts as context. In Phase 1 this is simple prompt context; later it could become a RAG layer with vector search. | New `knowledge/` directory + prompt injection logic |
| Brain | Extend instrument profiles (`profiles/*.json`) with references to knowledge entries, so the coach can surface relevant tips based on what the student is working on. | Schema extension to existing profiles |
| Face | Add "Did you know?" or "Technique tip" panel that surfaces knowledge entries during and after practice. | 1 new React component |
| Cloud | Content partnership pipeline (Kris + university colleagues contribute entries). | Editorial process, not engineering |

**Phase 1 impact:** Minimal. We can start populating the knowledge base during Phase 1 as a content task. The prompt injection is a small addition to the existing LLM coaching engine.

---

### Module 3: AI Listening Coach

**What it is:** The AI references real recordings to illustrate concepts. "Hear how Wynton Marsalis phrases this passage" — student taps, hears the excerpt, tries again.

**Architecture impact:**

| Layer | Change | Scope |
|---|---|---|
| Ears | None | — |
| Brain | Add a cross-genre reference database mapping techniques → real-world examples across genres. This feeds into coaching prompts so the LLM can say "this chromatic run shows up in Kendrick Lamar's horn arrangements." | New `references/` data layer (JSON: technique → artist → track → timestamp) |
| Brain | Audio excerpt management: store short clips (10–30 seconds) with metadata. Initially public domain + Creative Commons; later licensed. | Supabase Storage or local cache |
| Face | Add audio player component that can play reference excerpts inline with coaching tips. Add "Listen" button next to coaching suggestions. | 1 new React component (audio player) + UI integration |
| Cloud | Audio storage + streaming. Content licensing pipeline (legal, not engineering). | Supabase Storage, standard CDN pattern |

**Phase 1 impact:** None. This is a premium-tier feature (Phase 3 in the platform strategy). However, the cross-genre reference database can start being built as content during Phase 1.

---

### Module 4: Teacher Curriculum Marketplace

**What it is:** Teachers create and sell interactive, AI-coached practice routines. "16 Weeks to All-State" — student buys it, the AI coaches them through it.

**Architecture impact:**

| Layer | Change | Scope |
|---|---|---|
| Ears | None | — |
| Brain | Add curriculum package format: ordered sequence of MusicXML scores + practice instructions + coaching rubric + pacing schedule. The existing Score Mode + LLM Coach execute each step. | New schema definition for curriculum packages |
| Face | Add marketplace browse/purchase UI to the Student View. Add curriculum builder/upload UI to the Teacher Dashboard. | 2–3 new React page components |
| Cloud | Supabase tables for marketplace listings, purchases, revenue splits. Stripe Connect for teacher payouts (70/30 split). Content moderation pipeline. | Standard marketplace backend |

**Phase 1 impact:** None. This is Phase 4 in the platform strategy (Months 9–15).

---

### Module 5: B2B Licensing & Brand Partnerships

**What it is:** White-label the technology for instrument manufacturers (Yamaha, Conn-Selmer), method book publishers (Hal Leonard, Alfred Music), and music retailers.

**Architecture impact:**

| Layer | Change | Scope |
|---|---|---|
| Ears | Package as a standalone Rust library with a clean C API for embedding. | API surface design + build configuration |
| Brain | Configurable branding in coaching prompts (partner's voice, not ours). Licensing endpoint for per-unit activation. | Prompt templating + license server |
| Face | White-label theming system (colors, logos, partner branding). | CSS variable overrides + config |
| Cloud | License management server. Usage reporting/analytics for partners. | New microservice (small) |

**Phase 1 impact:** None. This is Phase 5 (Months 12–24). However, keeping the Rust crates cleanly separated (which we're already doing) makes future packaging much easier.

---

### Cross-Genre Contextual Coaching (philosophy, not a module)

**What it is:** The coaching voice understands that music doesn't exist in genre silos. When a clarinet student nails a chromatic passage, the AI connects it to Kendrick Lamar's horn arrangements.

**Architecture impact:**

| Layer | Change | Scope |
|---|---|---|
| Ears | None | — |
| Brain | Add student musical profile (onboarding: "what genres do you listen to?", "who are your favorite artists?"). Store in SQLite. Feed into LLM system prompts. | 1 new SQLite table + onboarding flow + prompt enrichment |
| Brain | Cross-genre reference database (same as Listening Coach Module 3 above). Maps techniques → genre examples. | Shared data layer with Module 3 |
| Face | Onboarding wizard for musical profile. Coaching tips now include cross-genre references. | 1 new onboarding component |

**Phase 1 impact:** Low. The student musical profile is a small addition to onboarding. The LLM system prompts can start referencing genres immediately — the cross-genre reference database enriches this over time but isn't required to start.

---

## New shared infrastructure (across modules)

These are the genuinely new pieces that don't exist in v2:

| Component | What it is | When needed |
|---|---|---|
| **Content packaging system** | A format for bundling MusicXML + metadata + coaching rubrics into distributable packs (audition prep, curricula). | Phase 2 |
| **Stripe integration** | Seasonal purchases, marketplace splits, B2B licensing fees. | Phase 2 |
| **Student musical profile** | Onboarding data about genre preferences, experience level, goals. Feeds LLM prompts. | Phase 1 (small) |
| **Cross-genre reference database** | Technique → genre → artist → example mappings. Shared by coaching engine and Listening Coach. | Content starts Phase 1; feature launches Phase 3 |
| **Audio player component** | Plays reference excerpts inline with coaching. | Phase 3 |
| **Marketplace backend** | CRUD + payments for teacher-created curricula. | Phase 4 |
| **License management** | Per-unit activation for B2B partners. | Phase 5 |

---

## Confirmation: Phase 1 is unaffected

The current Phase 1 spec backlog builds:

- Phrase-level analysis engine (Brain)
- LLM coaching with whispered tips and session recaps (Brain)
- Score following via Matchmaker (Brain)
- OSMD score rendering (Face)
- Free Play and Score Mode (Face)
- MusicXML import (Brain)
- Brass + voice + piano profiles (Ears/Brain)
- SQLite session history (Brain)
- Offline fallback (Brain)

**None of these change.** The platform strategy adds revenue modules and content layers that sit on top of Phase 1's foundation. The only Phase 1 addition worth considering is the student musical profile (a small onboarding form + 1 SQLite table + a few lines in the LLM system prompt). This is optional and can be added as a late Phase 1 story or deferred to Phase 2.

**Keep building. The specs are good.**

---

## Updated roadmap alignment

| Platform Strategy Phase | Architecture Phase | Timeline | What ships |
|---|---|---|---|
| Phase 1 — Foundation | Architecture Phase 1 (MVP) | Months 1–6 | Core practice companion + LLM coaching + cross-genre prompts |
| Phase 2 — Audition Prep | Architecture Phase 2 (Smart Import + Tone) | Months 4–8 | Audition Simulator + content packs + Stripe |
| Phase 3 — Knowledge + Listening | Architecture Phase 2–3 | Months 6–12 | Knowledge Engine + Listening Coach + premium tier |
| Phase 4 — Marketplace | Architecture Phase 3 (Teacher Platform) | Months 9–15 | Teacher Marketplace + curriculum builder |
| Phase 5 — B2B + Scale | Architecture Phase 3+ | Months 12–24 | White-label SDK + licensing |

The architecture phases and platform strategy phases are **not 1:1** — the platform strategy's phases overlap and interleave with the architecture phases. This is by design. The architecture phases are about technical capability; the platform strategy phases are about market timing.
