# Design Decisions Log

A running record of the non-obvious product + engineering calls we've made, and why. New engineers should read this before touching the relevant subsystems so they don't re-open settled questions.

Format: one entry per decision. Date, what we decided, what we rejected, the reasoning. Terse on purpose.

---

## 2026-04-20 — Single-mic ensemble: group-level feedback only, never per-player

**Decision:** Musa's ensemble / multi-person practice mode (story [#40](https://github.com/perice-pope/ai-music-companion/issues/40)) will give **group-level** feedback from a single mic. It will NOT attempt to give individual feedback to each player in the mix.

**Rejected alternatives:**
- **Per-player mics.** Bleed is real; socially weird; requires each musician to wear a lav. Kills the "just show up and play" vibe.
- **Single-mic real-time source separation of same-instrument players.** Not a solved problem as of 2026. The stem separators on the market (Spleeter, Demucs, RipX, LALAL) split audio into pre-trained timbre categories (vocals, drums, bass, other) — they work because those categories are timbrally distinct, trained on huge labeled corpora. Two trumpets playing harmony have the same timbre — they blend into a new combined sound, which is musically the whole point. Even hypothetical best-in-class research can't separate them reliably, and none of it is real-time productized.
- **Remote multi-device live sessions.** Network audio latency makes this a bad product regardless of the separation question. Different category of app entirely (see JamKazam, Jammr, Sonobus — all struggled). Not what Musa is.

**What we CAN do technically:** Polyphonic pitch detection (e.g., Spotify's Basic-Pitch) tells us *what pitches are in the mix right now* — just not who's playing which. That's enough for group-level coaching: intonation center of the section, entrance alignment, harmonic integrity, rhythmic tightness.

**Why this is the right product call anyway:** A choir director or band teacher doesn't give per-player feedback in a group rehearsal — they conduct the ensemble as a whole. "Sopranos drifted sharp in bar 12." "Brass came in half a beat late on the entrance." Group-level feedback is what the target users actually want. The technical constraint and the product reality align.

**Consequence:** The UI wording in ensemble mode must never imply we can isolate individuals. "The group sagged on the release." Never "Sarah, you were flat."

**Related:** [Story #40](https://github.com/perice-pope/ai-music-companion/issues/40), [Story #14 design §2.5](./story-14-free-play-mode.md).

---

## 2026-04-20 — Participants + segments data model, even in solo MVP

**Decision:** The session data model is `Session → Vec<Participant> → Vec<InstrumentSegment>`, even though the solo MVP only ever populates a single participant with one or more segments.

**Rejected alternative:** A flat `Session → Vec<PhraseSummary>` with `instrument: String` — what `SessionRecorder` currently has.

**Reasoning:** Supports multi-instrumentalists switching mid-session (solo Phase 1 feature) AND ensemble mode (story #40) with zero data-model changes later. Cost is one extra PR of refactoring now (~500 lines, low risk because it's pure backend). Avoids a painful 3-4 day retrofit of `SessionRecorder`, `SessionStore`, `StoredSession`, and every recap path at a later point when we've accumulated more consumers.

"Cheap insurance, paid upfront."

**Consequence:** PR 0 of the free-play implementation is the data model migration. No UI changes in that PR — purely refactoring `crates/brain` + SQLite migration.

**Related:** [Story #14 design §2.5](./story-14-free-play-mode.md), [Hotspot #32 scoring.rs refactor](https://github.com/perice-pope/ai-music-companion/issues/32) (should consume phrase-level stats per-segment, not per-session).

---

## 2026-04-20 — No pause, no resume — sessions are one-shot

**Decision:** A practice session runs wall-clock from `start_practice_session` to `end_practice_session`. No pause button. If the user walks away for 3 minutes, that's 3 minutes of session time.

**Rejected alternative:** Pause/resume state with "true practice time" tracked separately from "session duration."

**Reasoning:** We don't have a reliable "user stopped playing" signal that doesn't false-trigger on rests in the music itself. Adding a manual pause button is UX complexity (paused-state rendering, resume flow, whether paused time counts toward coaching "context" duration, etc.) disproportionate to the rare "I need to take a phone call" use case. Yoga-class framing, not gym-app framing.

**Consequence:** Session recap "duration_secs" is elapsed wall time, not an estimate of active playing. LLM prompt includes duration so it can say things like "for a 45-minute session you covered a lot of ground" — if that's literal wall-clock time, fine.

**If users ask for pause later:** Add it as a story. Easy enough to retrofit; not worth paying for now.

**Related:** [Story #14 design §2](./story-14-free-play-mode.md).

---

## 2026-04-20 — Coaching-off-with-banner, not rule-based filler tips

**Decision:** If no LLM API key is configured, coaching is visibly unavailable — the tip panel becomes a thin "Coaching unavailable — add API key to enable" rail. The session still works fully otherwise.

**Rejected alternative:** A local rule-based tip generator that produces generic encouragement ("Nice tone!" / "Keep it up!") so users see *something* even without a key.

**Reasoning:** Repeated generic tips actively feel more broken than no tips. The user notices the pattern within two or three tips and loses trust in the coaching signal — which then poisons the real LLM coaching when they eventually add a key, because they've been trained to dismiss the panel.

Silence is honest. Generic filler is a little lie that compounds.

**Consequence:** First-run UX without a key is "here's a pitch meter + recap" — functional but not magical. Documented in README as the env-var flow. Worth a product-side decision later whether to ship a key by default on the marketing download.

**Related:** [Story #14 design §4](./story-14-free-play-mode.md), [Hotspot #33 LLM coaching skeleton](https://github.com/perice-pope/ai-music-companion/issues/33).

---

## 2026-04-20 — PR 0 scope: in-memory data model first, SQLite schema grows later

**Decision:** PR 0 lands the new `Participant → InstrumentSegment` shape in the Rust types AND on the `CompletedSession` public surface, but does NOT grow the SQLite schema with new `participants` / `instrument_segments` tables. Persistence still flows through the unchanged `SessionRecap` JSON blob stored on the existing `sessions` row.

**Rejected alternative:** Do the schema migration in this PR too, as the original brief described. ~200 extra lines of SQL + migration + tests + round-trip fidelity coverage.

**Reasoning:** The schema change is an *optimization* on top of a working system. The recap blob already preserves what a history UI needs (phrase count, duration, instrument, tips). Growing the schema lets us query phrases directly (useful for analytics / progress dashboards), but NO current consumer needs that — story #17 (practice history dashboard) is the first one that would, and it's not started yet. Shipping a smaller, lower-risk PR 0 unblocks PR 1-3 of the free-play feature sooner; the schema PR becomes a targeted follow-up that gets its own review focus.

**Consequence:** 
- A completed session round-trips through JSON, not through typed SQL rows. If you're reading phrase stats for analytics, you pay a JSON parse.
- When we ship the schema growth PR later, the `load()` path will prefer typed tables when present and fall back to lazy JSON parse for old-shape rows.
- No DB migration risk in this PR — `sessions` schema unchanged.

**Related:** [Story #14 design §2.5](./story-14-free-play-mode.md), Story #17 (practice history UI) will trigger the follow-up schema PR.

---

## 2026-04-23 — CRM is off-the-shelf (Airtable / Attio / Pipedrive), not a custom build

**Decision:** Sales lead tracking and outbound pipeline management lives in an **off-the-shelf CRM** (Airtable as the likely v0, upgrading to Attio or Pipedrive if the sales workflow outgrows it). We are **not** building a leads tracker as another page on the pitch site, even though the infrastructure (Supabase + GitHub Pages + magic-link auth) would make it cheap to start.

**Rejected alternative:** A `/leads/` page in the `ai-music-companion-pitch` repo, auth-gated, writing to new `leads` / `lead_activities` / `team_members` tables in the existing Supabase project. Inbound `pilot_signups` rows would auto-create leads via an Edge Function. Proposed schema and UI were scoped in the conversation that produced this entry.

**Reasoning:**
- Building a CRM is a **classic founder trap**. The domain is deceptively deep — email sync, calendar sync, activity logs, reporting, mobile, bulk import, deduplication, pipeline configurability. We'd spend months chasing parity with tools that cost $25–40/mo/seat and work today.
- **Sales muscle memory matters.** A competent salesperson already has a CRM workflow. Making them learn our table costs momentum we don't have. Their tool preference is a data point, not a design constraint we fight.
- **Engineering time is the scarce resource.** Every hour on CRM internals is an hour not on the Rust core, Eyes, or mobile. Leads tooling is not a moat.
- **Cost math doesn't justify a build.** At 2 seats the savings are ~$50/mo — a rounding error vs. the engineering opportunity cost. Only at ~10+ seats does the math get interesting, and by then we'll know exactly what we need and can build deliberately.
- **Off-the-shelf still integrates.** Airtable has a REST API; Attio/Pipedrive have webhooks. Inbound `pilot_signups` → CRM pipe can be wired with a 20-line Supabase Edge Function if/when we want that automation.

**What we DID keep from the design discussion:** the schema shape (`org_type`, `instrument_focus`, `student_count`, `status`, `next_action_date`, etc.) is a useful reference for configuring whichever tool we pick. Don't invent a new schema from scratch — start from the one in the conversation that produced this decision.

**Consequence:**
- Any future "I want a dashboard showing how sales is going" ask is answered by the CRM's reporting, not by us building a page.
- If the sales flow outgrows Airtable's view/filter ergonomics, the next stop is **Attio or Pipedrive**, not a custom build. The trigger for revisiting "build it ourselves" is a concrete capability the off-the-shelf tool cannot support, not general dissatisfaction.
- Inbound `pilot_signups` currently stays in Supabase; any CRM integration is wired as an explicit follow-up, not assumed.

**Related:** [`ai-music-companion-pitch/README.md`](../../ai-music-companion-pitch/README.md) (pitch site + signup backend), `pilot_signups` table in the Supabase `musa` project.

---

## 2026-04-25 — Score follower implementation: Rust port of Online DTW

**Decision:** Implemented score following using a **Rust port of Online Dynamic Time Warping (DTW)**, not a PyO3 bridge to Matchmaker. The Rust implementation is now shipping in `crates/brain/src/follower.rs`.

**Rejected alternative:** PyO3 bridge to the Matchmaker library (battle-tested Online DTW from a Python package, ~2 dev-days). This would have added a Python runtime dependency to the Tauri bundle.

**Reasoning:**
- **Self-contained binary:** No Python runtime in the ship, simplifying distribution and reducing binary size.
- **Performance exceeds spec:** Latency benchmarks show ~170ns per alignment step (p50), well under the 3ms budget even with headroom. Single event latency measured at 168.12 ns (p50) with consistent sub-microsecond performance.
- **Code ownership:** We maintain and understand the DTW implementation directly. No external Python dependency means no version compatibility headaches.
- **Integrated smoothly:** The Rust implementation integrates with the existing Rust audio pipeline (`AudioEvent` → `ScorePosition`) without FFI overhead.

**What the implementation provides:**
- Online DTW with windowed cost matrix (space-efficient, real-time suitable)
- Silence-gap auto-reset (detects rehearsal breaks, re-aligns on new entry)
- Tempo tolerance (±20%) for performances that speed up/slow down
- Per-measure and per-beat position tracking
- Comprehensive test suite covering in-order playback, out-of-order recovery, and edge cases

**Consequence:**
- Future Rust developers touching `crates/brain/src/follower.rs` should understand DTW basics (see comments in the implementation).
- If performance characteristics change (longer scores, more complex alignment), profile with the `cargo bench --bench score_follower_latency` benchmark before optimizing.
- The `PhraseAggregator` now optionally uses score positions to segment phrases at measure boundaries, in addition to silence gaps. This is backward-compatible — scores are optional.

**Related:** [Hotspot #34](https://github.com/perice-pope/ai-music-companion/issues/34) (score follower implementation), `crates/brain/benches/score_follower_latency.rs` (latency verification).

---

## 2026-07-08 — Stem separation runs locally (Demucs-class), never in the cloud (v1)

1. **Decided:** full-mix audio imports get on-device stem separation (Demucs-class
   quality bar), with the player choosing which stem to practice. No cloud separation
   tier in v1.
2. **Rejected:** cloud separation API (quality + zero install cost, but violates the
   offline-first default and ships user audio off-device); bundling model weights in
   the installer (300 MB class — installer bloat for a feature many won't touch).
3. **Reasoning:** practice audio is the most private thing users give us; the
   offline-first promise ("the internet is NEVER required for core value") is the
   product's spine. We already run ONNX models on-device (`crates/transcribe` via
   `ort` load-dynamic + vendored runtime), so local inference is our paved road.
   Weights download once, on first use, behind an explicit disclosed prompt.
4. **Watch out:** htdemucs's ONNX export (transformer + iSTFT) is known-painful —
   the spec's slice-2 spike benches it against Open-Unmix and may land Open-Unmix
   first with the same architecture. The DECISION (local, on-device) stands
   regardless of which model ships. Also: separation is CPU-heavy; it must run off
   the main thread (#323's pattern) with progress + cancel.
5. **Related:** #328 (story), `docs/specs/328-stem-separation.md`, #324 (the 4/8-notes
   silent fail), #267 (calm degradation), #323 (off-main-thread imports).

---

## How to add to this log

When you make a design call that a future reader would want the reasoning for — add an entry here. Include:

1. **What we decided** — one sentence, the actual call
2. **Rejected alternatives** — what we considered and said no to
3. **Reasoning** — why. Honest. No hedging.
4. **Consequence** — what the reader should watch out for
5. **Related** — link issues, PRs, other design docs

Keep it terse. This document gets long fast if we ramble.
