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

## How to add to this log

When you make a design call that a future reader would want the reasoning for — add an entry here. Include:

1. **What we decided** — one sentence, the actual call
2. **Rejected alternatives** — what we considered and said no to
3. **Reasoning** — why. Honest. No hedging.
4. **Consequence** — what the reader should watch out for
5. **Related** — link issues, PRs, other design docs

Keep it terse. This document gets long fast if we ramble.
