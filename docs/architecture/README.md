# Architecture

This folder holds the living architecture for Musa / AI Music Companion.

## Start here (a new engineer's reading order)

1. [`../../CLAUDE.md`](../../CLAUDE.md) — house rules, repo layout, gates, Definition of Done.
2. [`rv-methodology.md`](./rv-methodology.md) — **the product north star.** The unit of practice
   is the cell rowed through 12 keys; key detection is display honesty only. Read before
   designing any practice feature.
3. [`offline-first-and-network-transparency.md`](./offline-first-and-network-transparency.md) —
   the offline promise and the complete enumeration of every networked feature.
4. [`../design/decisions-log.md`](../design/decisions-log.md) — settled product + engineering
   calls. Silence there means "not decided yet."
5. The latest `cto-audit`-labeled issue — a code-verified map of what is actually built,
   refreshed on a weekly schedule (check its date; the cadence sometimes skips a week).

Do **not** start from `architecture-v2.md` — it is stamped historical (see below).

## Files

### Current

| File | What it is |
|------|------------|
| [`rv-methodology.md`](./rv-methodology.md) | **The product north star.** The Random Variations method: cell × 12-tone row × modifiers, and what key detection is (and is not) for. |
| [`offline-first-and-network-transparency.md`](./offline-first-and-network-transparency.md) | The offline-by-default contract: the core loop needs zero network; every networked feature is opt-in, off by default, and enumerated here. |
| [`network-call-sites.allowlist`](./network-call-sites.allowlist) | Machine-checkable registry of the only source files allowed to contain outbound-network call sites (enforced by `scripts/check_network_disclosure.sh`). |
| [`piece-identification.md`](./piece-identification.md) | Design doc: "know the piece, listen accordingly" — library match first, via the DTW follower flipped into retrieval (#214, #208, #417 item 5). |
| [`score-import-and-transcription.md`](./score-import-and-transcription.md) | Decision doc: how PDFs/photos (OMR) and audio (on-device basic-pitch) become scores; MusicXML is the canonical internal format. |
| [`teacher-dashboard-datamodel.md`](./teacher-dashboard-datamodel.md) | BI spec for the teacher dashboard (#449); the cloud star schema + RLS are landed in `supabase/migrations/`. |
| [`teacher-audit.md`](./teacher-audit.md) | RFC: opt-in session-audio capture for human (teacher) review — the privacy precedent other cloud features follow. |
| [`eyes.md`](./eyes.md) | RFC (exploration): computer vision as a third sensor for technique analysis — landmarks live, VLM async. |
| [`mobile.md`](./mobile.md) | RFC (committed): iOS + Android via Tauri 2. iPad-first for schools, then phone. |
| [`sdlc-automation-loop.md`](./sdlc-automation-loop.md) | How we build: daily engineering agent, weekly CTO audit, testing standards, PR hygiene. |

### Drafts (partly aspirational — check the code before trusting a claim)

| File | What it is |
|------|------------|
| [`platform-spines.md`](./platform-spines.md) | Draft: how the platform's modules stay one system (overview of the three spines below). |
| [`platform-spine-personalization.md`](./platform-spine-personalization.md) | Draft spine, partially real: `MusicalFingerprint` (`crates/brain/src/fingerprint.rs`, migration `0004`) implements what it anticipates. |
| [`platform-spine-commerce.md`](./platform-spine-commerce.md) | Draft spine, aspirational: commerce/entitlements (#479) — no matching code yet. |
| [`platform-spine-content-format.md`](./platform-spine-content-format.md) | Draft spine, aspirational: unified content format — no matching code yet. |
| [`platform-modules-addendum.md`](./platform-modules-addendum.md) | Addendum to v2 — modular platform thinking (teacher/student/group modes, tone-quality model, etc.). |

### Historical (kept for decision tracking — not the current product)

| File | What it is |
|------|------------|
| [`architecture-v2.md`](./architecture-v2.md) | The April 2026 "coach, don't judge" spec. **Stamped HISTORICAL 2026-08-27**: CTO audits #363 and #494 found it "describes a different app" (#508 confirmed the drift) — the RV method it never mentions is now the dominant product surface, and several named tools were replaced or never built. The audits are the accurate map. |
| [`architecture-v1.md`](./architecture-v1.md) | Original pre-pivot architecture. Kept for diff / decision tracking. |
| [`research-notes.md`](./research-notes.md) | Early product + market research that fed into v1/v2. |

## Related

- [`../design/`](../design/) — per-story design docs (e.g. `story-14-free-play-mode.md`)
- [`../design/decisions-log.md`](../design/decisions-log.md) — running record of non-obvious product + engineering calls
- [`../specs/`](../specs/) — per-slice specs (`_TEMPLATE.md` is the required starting point)
- [`../testing-standards.md`](../testing-standards.md) — what "a real test" means here
- [`../../CLAUDE.md`](../../CLAUDE.md) — project rules the agents (and humans) follow
