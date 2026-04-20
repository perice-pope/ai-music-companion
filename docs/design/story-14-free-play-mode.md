# Story #14 — Free Play Practice Mode: Design Proposal

**Status:** Approved — implementation-ready
**Author:** Design proposal generated for founder + CTO review
**Target story:** [#14 Phase 1: Free play practice mode (no score)](https://github.com/perice-pope/ai-music-companion/issues/14)
**Dependencies landed:** #11 (PhraseAggregator), #12 (CoachingEngine), #13 (SessionRecorder + recap)

**Revision notes (2026-04-20):**
- ✅ Coaching-off-with-banner confirmed (§4)
- ✅ No-pause confirmed (§2)
- ❌ "Instrument locked for the session" **overruled** — multi-instrumentalists need to switch mid-session. Data model generalized to support participants + instrument segments (§2.5). Group practice / ensemble mode spun off as its own story: [#40 Ensemble practice mode](https://github.com/perice-pope/ai-music-companion/issues/40).
- See [design-decisions log](./decisions-log.md) for the rationale behind the single-mic ensemble scope call.

---

## 1. Product framing

### What free play is (user's POV)

A musician opens Musa, picks their instrument from a grid of cards, clicks **Start Practice**, and plays. Musa listens silently. Between phrases — the natural breath points — a short coaching tip appears off to the side: "Nice tone on that one. Try letting the line breathe at the end." The musician keeps playing. A session timer ticks in the corner. When they're done they click **End Session** and get a recap that reads like handwritten notes from a teacher: what went well, what to work on, what to try next time. Then they go back to the instrument picker and do it again tomorrow.

### Why this is the flagship Phase 1 feature

- It is the **smallest complete slice** of the "coach, don't judge" vision. No score parsing, no OSMD, no OMR, no matchmaker — just mic → phrases → LLM coach → recap. Everything downstream of Phase 1 builds on this loop.
- It is the **proof-of-differentiation**. Tonestro and SmartMusic cannot do this: their UX *requires* a score. Free play is how we demo the product to a skeptical musician in 90 seconds.
- It **exercises every layer** (Ears, Brain, Face) end-to-end for the first time, so every latency, IPC, and state-management assumption gets stress-tested against a real user loop.

### Three UX decisions that reinforce "coach, don't judge"

1. **No traffic lights, not even subtly.** The pitch trace is a single neutral color. No "you were 12 cents flat" badge. No score percentage anywhere on the recap screen. If a dev reviewer asks "can we just show the cents deviation as a small number", the answer is no — that's the exact wedge that becomes creeping note-grading.
2. **Tips are side-panel, never modal.** A modal stops the musician. A subtitle-style overlay reads as a verdict. A side panel reads as a post-it from a teacher you can glance at or ignore. We explicitly **do not** interrupt playing.
3. **Recap opens with strengths, not errors.** The recap component renders `strengths` above `areas_to_improve`. This is a layout decision encoded in the component, not just a prompt suggestion — the UI itself enforces the tone.

Things we are rejecting: streak counters, "session score" numbers, per-phrase thumbs up/thumbs down buttons, emojis in tips. Anything gamified is out.

---

## 2. Frontend architecture

### Route / app shell changes

**Recommendation: Zustand enum, not react-router.** Free play is three screens (selector → session → recap) that share a single store and never need URLs, deep links, or browser history. Adding `react-router` for three states is overkill and couples us to browser navigation semantics in a Tauri webview.

Add to `practiceStore`:

```typescript
type AppScreen = "selector" | "session" | "recap";
```

`App.tsx` becomes a thin switch on `screen`. If we later grow to 6+ screens or want URL-based deep linking (unlikely for a desktop practice app), we migrate then.

### Component tree

```
<App>
  └── <PracticeShell>                      // Top-level screen router (switch on screen enum)
      ├── <InstrumentSelector>             // EXISTS — card grid; "Start Practice" becomes enabled
      ├── <PracticeSession>                // NEW — active session view
      │    ├── <PitchDisplay>              // EXISTS — reused as-is, no color judgments
      │    ├── <SessionTimer>              // NEW — MM:SS elapsed, ticks via setInterval
      │    ├── <CoachingTipPanel>          // NEW — side panel, slides in, auto-dismisses
      │    └── <EndSessionButton>          // NEW — tiny, lives at top-right
      └── <SessionRecap>                   // NEW — strengths / areas / next-session; "Done" → selector
```

Seven components. `PracticeShell` is the new `App.tsx` body. `PitchDisplay` and `InstrumentSelector` already exist from Phase 0 and need minor adaptation (selector gets a "Start Practice" CTA; pitch display reads from `practiceStore.isListening` instead of `audioStore.isListening`, or both via a subscription adapter — see store design below).

### Zustand store shape

**Recommendation: a new `practiceStore`, keep `audioStore` narrow.**

`audioStore` already handles raw audio events and instrument selection. Keeping it focused on "what the Ears layer is saying right now" is the right layering. A new `practiceStore` owns the session lifecycle, coaching queue, and screen routing.

```typescript
// apps/desktop/src/stores/practiceStore.ts
import type { PhraseSummary, CoachingTip, SessionRecap } from "../types/brain";

export type AppScreen = "selector" | "session" | "recap";
export type SessionStatus = "idle" | "starting" | "listening" | "ending" | "recap_ready";

export interface QueuedTip {
  id: string;              // UUID so React keys are stable across re-renders
  tip: CoachingTip;
  receivedAt: number;      // ms since epoch, for auto-dismiss timer
  phraseIndex: number;
}

export interface PracticeState {
  // Routing
  screen: AppScreen;

  // Session lifecycle
  status: SessionStatus;
  sessionId: string | null;           // opaque string from Rust
  instrumentName: string | null;      // duplicated from audioStore for convenience
  startedAtMs: number | null;         // wall-clock, for timer
  elapsedSecs: number;                // ticked by setInterval, not derived on render

  // Live session data
  phrases: PhraseSummary[];
  tipQueue: QueuedTip[];              // currently-visible tips (max 1 shown, rest queued)

  // Recap
  recap: SessionRecap | null;
  recapError: string | null;

  // UI prefs (persisted)
  coachingEnabled: boolean;

  // Actions
  startSession: (instrument: string) => Promise<void>;
  endSession: () => Promise<void>;
  pushPhrase: (phrase: PhraseSummary) => void;
  pushTip: (tip: CoachingTip, phraseIndex: number) => void;
  dismissTip: (id: string) => void;
  tick: () => void;                   // called every 1s while listening
  returnToSelector: () => void;       // from recap back to home
  setCoachingEnabled: (on: boolean) => void;
}
```

**Why this shape:**
- `status` is a finite state machine, not a pile of booleans. It prevents "listening but no sessionId" bugs.
- `tipQueue` is a FIFO, not a single current tip. If two phrases finish 400ms apart, we don't want to drop the second tip — we queue it and show it after the first dismisses.
- `elapsedSecs` is stored, not derived. Derived-from-`Date.now()`-on-render causes the whole tree to re-render 60× a second if anything subscribes. A 1Hz `tick()` is cheap and correct.
- `coachingEnabled` is persisted (localStorage) so the user's choice survives app restart.

### Coaching tip display: how do tips appear?

**Pick: side panel, fixed right, slides in from the right edge.** A single card at a time, 10-second auto-dismiss, stackable queue behind it (with a small "+2" indicator if tips queue up).

**Why side panel over alternatives:**
- **Rejected: modal.** Interrupts the musician. Violates "never stop the music."
- **Rejected: toast (bottom, auto-dismiss).** Too ephemeral — musicians won't look away from their instrument every 10 seconds to catch something that disappears. A persistent-until-read card is more respectful of attention.
- **Rejected: subtitle at bottom of screen.** Reads like a verdict/scoreboard, not a teacher's note. Subtitles imply "this is what just happened." A sidebar card implies "here's a thought when you have a second."
- **Rejected: inline above pitch display.** Pushes the pitch trace around, distracting during playing.

Side panel lives at fixed right, 320px wide, collapsible with a small toggle. When coaching is disabled, the panel shrinks to a thin "Coaching off" rail.

### Session timer UX

- **Location:** Top-right of `PracticeSession`, small, monospace, same visual weight as a window title. Not the hero.
- **Tick rate:** 1Hz via `setInterval`. The store holds `elapsedSecs: number`; the component displays `MM:SS`. A 1Hz update across a number in a corner will not drop frames even on a weak machine.
- **Pause behavior:** **v1 does not pause.** The timer runs from `startSession` to `endSession`. If the musician puts the instrument down for 30s to check a text, that counts as session time. Rationale: (a) we don't have a reliable "user stopped playing" signal that won't false-trigger on rests, (b) pausing adds UX complexity (resume button, paused-state rendering) disproportionate to v1. If users ask for a pause button later, add it as a story.

### Recap screen

Shows, top-to-bottom:

1. **Header:** "Nice session." Instrument name. Duration in plain English ("28 minutes"). Date.
2. **Overall assessment:** One paragraph from `recap.overall_assessment`. Large, readable, serif font to feel like a handwritten note.
3. **Strengths** (rendered FIRST, deliberately): bulleted list from `recap.strengths`.
4. **Areas to work on**: bulleted list from `recap.areas_to_improve`. Same visual weight as strengths — not red, not warned.
5. **Next time, try**: bulleted list from `recap.next_session_suggestions`.
6. **Two buttons at the bottom:** `[Practice again]` (returns to `selector`) and `[Done]` (also returns to selector — "done" is framed as a positive completion, not an exit).

If `recap` is null and `recapError` is set: show a calm error card ("I had trouble generating your recap, but your session is saved. Here's what you played: N phrases over M minutes.") plus the same two buttons. Never fail loudly on a recap. The session still happened.

---

## 2.5 Participants and instrument segments (data model)

Free play MVP ships as **solo + mid-session instrument switch**. But the data model is built to also carry **ensemble mode** (story #40) without a retrofit later. Getting this right now is cheap; retrofitting it later is ruinous.

### The shape

```
Session
├─ id, started_at, ended_at
├─ participants: Vec<Participant>          ← length 1 in MVP
│  └─ Participant
│     ├─ id (stable within the session)
│     ├─ display_name ("You", "Student A", "The group")
│     ├─ input_source: InputSource          ← MVP: always DefaultMic
│     └─ segments: Vec<InstrumentSegment>
│        └─ InstrumentSegment
│           ├─ instrument: InstrumentProfile
│           ├─ started_at, ended_at (Option<> — open segment has None)
│           ├─ phrases: Vec<PhraseSummary>
│           └─ coaching_tips: Vec<RecordedTip>
```

### Why this shape

- **Solo with switching** (the MVP user): `participants.len() == 1`, that participant accumulates multiple `InstrumentSegment`s over time. A trumpet warm-up segment, then a piano segment, then a vocal segment — all one session.
- **Ensemble** (future, story #40): `participants.len() == 1` still — because a single mic cannot reliably separate same-instrument players in real time (see [decisions-log](./decisions-log.md)). The *group* is modeled as one participant with a `display_name` like "The group" and a specialized instrument profile describing the ensemble. No data model change needed.
- **Recap** iterates participants → segments → phrases. Works identically for both shapes.
- **Coaching** runs per-segment — a trumpet segment gets trumpet-flavored tips, a piano segment gets piano-flavored tips. This is actually *more correct* than treating a switched session as one undifferentiated blob.
- **SQLite schema** grows by two small tables (`participants`, `instrument_segments`). The existing `sessions` row stays unchanged; phrases and tips migrate onto the segment row.

### UX consequence

The `<InstrumentSelector>` stays as-is for session start. Inside `<PracticeSession>`, the current instrument name in the header is a button — clicking it opens a small "switch instrument" menu. Confirming closes the current segment and opens a new one:

```
┌─────────────────────────────────────┐
│ Trumpet ▾     ● 04:23       [End]   │  ← header
│                                     │
│     (pitch display)                 │
│                                     │
└─────────────────────────────────────┘
```

When switched, the tip panel briefly shows a small "You're on piano now" acknowledgement (not a tip — a status beat). The aggregator flushes any open phrase and resets. The new segment's pitch-detection profile is reconfigured (frequency range, vibrato tolerance, attack settings).

### Scope reminder

- **MVP ships:** solo + segment switching. One participant always.
- **MVP does NOT ship:** per-participant UI, add-a-participant button, multi-channel audio routing, polyphonic pitch detection. All of that is [#40](https://github.com/perice-pope/ai-music-companion/issues/40).

---

## 3. Backend wiring (Rust side)

### Tauri commands (the full surface)

```rust
// apps/desktop/src-tauri/src/commands.rs

/// Start a new practice session. Loads the instrument profile,
/// spins up the audio capture + phrase aggregator + coaching task,
/// returns the session id.
#[tauri::command]
async fn start_practice_session(
    instrument: String,
    coaching_enabled: bool,
    state: State<'_, AppState>,
) -> Result<String, String>;  // Ok = SessionId as string

/// End the active session. Stops audio capture, flushes the aggregator,
/// persists the completed session, kicks off recap generation.
/// Returns the SessionRecap when ready (may be up to 30s on cold LLM call).
#[tauri::command]
async fn end_practice_session(
    state: State<'_, AppState>,
) -> Result<SessionRecap, String>;

/// Switch the active participant's instrument mid-session. Closes the
/// current InstrumentSegment, opens a new one, reconfigures pitch
/// detection for the new profile. Returns the new segment id.
#[tauri::command]
async fn switch_instrument(
    instrument: String,
    state: State<'_, AppState>,
) -> Result<String, String>;  // Ok = new SegmentId as string

/// List instrument profile names available on disk.
/// Used to populate the selector instead of hardcoding in TS.
#[tauri::command]
fn list_instruments() -> Result<Vec<InstrumentInfo>, String>;
```

That's it — four commands. Every other piece of data flows over events, not commands.

**Explicitly collapsed:**
- No separate `configure_detection(instrument)` — the profile load happens inside `start_practice_session` (first segment) and `switch_instrument` (subsequent segments).
- No separate `get_session_state()` poll — state lives in the Zustand store, driven by events.
- No `pause_session` / `resume_session` — out of scope for v1 (see §6).
- No `add_participant` — ensemble mode (story #40) adds this when it lands; the data model already supports it, the command surface doesn't yet expose it.

### Tauri events emitted

| Event name | Payload | When | Consumer |
|---|---|---|---|
| `audio-event` | `AudioEvent` (existing) | ~every audio frame (~20ms) | `PitchDisplay` |
| `phrase-detected` | `PhraseSummary` | When aggregator closes a phrase | `practiceStore.pushPhrase` |
| `coaching-tip` | `{ tip: CoachingTip, phrase_index: usize }` | When coaching task returns a tip | `practiceStore.pushTip` |
| `session-status` | `{ status: "starting" \| "listening" \| "ending" \| "coaching" }` | State transitions | `practiceStore.setStatus` — used to show "still thinking..." during recap |
| `segment-changed` | `{ segment_id: String, instrument: String, started_at: String }` | `switch_instrument` succeeded | `practiceStore.setActiveSegment` — updates header and flushes current-phrase state |

`session-recap` is **not** an event — it's the return value of `end_practice_session`. Returning it as a command result makes the error path cleaner (frontend `await`s the recap directly).

Reuse: `AudioEvent`, `PhraseSummary`, `CoachingTip`, `SessionRecap` all exist in Rust with serde-derive. Mirror types live in `apps/desktop/src/types/brain.ts` — single hand-maintained file, kept in sync by an integration test that JSON-roundtrips a fixture.

### State machine

```
                     start_practice_session()
       ┌──────────┐ ──────────────────────► ┌───────────┐
       │          │                         │           │
       │   Idle   │                         │ Starting  │
       │          │ ◄──────┐                │  (setup)  │
       └──────────┘        │                └─────┬─────┘
            ▲              │                      │ profile loaded
            │              │                      │ stream started
            │              │                      ▼
            │              │                ┌───────────┐
            │              │                │           │ ◄─── AudioEvent
            │              │                │ Listening │ ◄─── PhraseSummary (async: CoachingTip)
            │              │                │           │
            │              │                └─────┬─────┘
            │              │                      │ end_practice_session()
            │              │                      ▼
            │              │                ┌───────────┐
            │              │                │  Ending   │ stream stopped,
            │              │                │ (flush +  │ aggregator flushed,
            │              │                │   recap)  │ RecapGenerator awaited
            │              │                └─────┬─────┘
            │              │                      │ recap ready (or error)
            │              │                      ▼
            │              │                ┌───────────┐
            │              └────────────────│   Recap   │
            │                   "Done"      │           │
            │                                └───────────┘
            │                                      │
            └──────────────────────────────────────┘
                  returnToSelector() (frontend only)
```

**Where each state lives:**
- **Rust owns:** `Idle` ↔ `Starting` ↔ `Listening` ↔ `Ending`. A single `AppState` struct with a `Mutex<Option<ActiveSession>>` makes the "one session at a time" invariant explicit.
- **Frontend owns:** `Recap` display and the `Recap → Idle` transition (`returnToSelector` is pure UI). Rust has no memory of being "in recap" — once `end_practice_session` returns, Rust is back to `Idle`. The frontend just hasn't navigated home yet.

### Background threads

Current (Phase 0): `audio thread (cpal) → ring buffer → processing thread (pitch) → main thread (Tauri emit)`.

**What #14 adds:**
1. **Phrase aggregator** runs on the existing processing thread. No new thread. The aggregator is cheap (a Vec push + periodic stats computation) and already designed to live off the audio thread.
2. **Coaching task** runs in a **tokio task** spawned by the Tauri runtime, one per phrase completion. Phrase aggregator produces a `PhraseSummary`, main-thread handler does `tokio::spawn(async move { engine.get_tip(...).await })`, result emits on `coaching-tip`. Rate limiting is the engine's responsibility (already implemented in `CoachingEngine`).
3. **Recap generation** runs in a tokio task when `end_practice_session` is invoked. The command `.await`s it and returns the result.

So: **zero new OS threads. Two new tokio tasks** (coaching-per-phrase, recap-on-end). This keeps us inside the "one audio thread, one processing thread, async for everything else" discipline.

---

## 4. Coaching integration

### Wiring the CoachingEngine

- A single `Arc<Mutex<CoachingEngine>>` lives in `AppState`, constructed at app launch (reads `MUSIC_COMPANION_LLM_API_KEY` from env via existing `CoachingEngine::new`).
- On `phrase-detected`, the main thread spawns a tokio task that locks the engine, calls `get_tip(&phrase, &context).await`, and emits `coaching-tip` on success.
- `SessionContext` is assembled from the `SessionRecorder` live state: `instrument`, `session_duration_secs` (now − started_at), `phrases_played` (count), `previous_tips` (last N=5 tips from `SessionRecorder.tips`, to avoid repetition).
- **Critical:** the Mutex is held only for the short `get_tip` await. If two phrases finish 100ms apart, the second tokio task blocks briefly on the lock — acceptable because the engine's internal rate limiter will short-circuit the second call anyway.

### Graceful degradation when `ANTHROPIC_API_KEY` is unset

**Recommendation: coaching is silently disabled, UI shows "Coaching off (no API key)" in the side-panel rail.** Concretely:

- At app launch, try `CoachingEngine::new(...)`. If it returns `CoachingError::MissingApiKey`, store `None` in `AppState.coaching_engine`.
- `start_practice_session` still succeeds. Phrase aggregation still runs. The session is fully functional minus tips.
- When the frontend calls `start_practice_session`, the response includes a `coaching_available: bool` field (or — simpler — a startup `get_app_capabilities` command). The tip panel renders "Coaching unavailable — add `MUSIC_COMPANION_LLM_API_KEY` to enable" instead of the panel.
- The **recap** path: if no engine, `end_practice_session` returns a minimal rule-based recap ("You played N phrases over M minutes. Your average pitch was... Keep practicing!") rather than erroring. This matches the "local fallback" promise in the architecture doc.

**Why not a rule-based tip fallback in free play itself?** The existing `CoachingEngine::fallback_tip()` returns a single generic string. Shown repeatedly, it feels broken. Better to be honest: "coaching is off" beats "same encouraging platitude every 30 seconds."

### Rate-limiting and "still thinking"

- The engine's `rate_limit_secs` (default 3.0) already short-circuits rapid calls. The short-circuit returns a `rate_limited_tip()` — a generic text — which is *also* not ideal if shown repeatedly.
- **Recommendation: when the engine returns a rate-limited tip, the backend silently drops it (does not emit `coaching-tip`).** The UI stays empty between real tips. Better silence than filler.
- For the real-LLM path: tips typically return in 0.5–2s. We do not show a spinner in the side panel — a spinner creates anticipation and judgment framing. If it arrives, it appears; if not, the musician keeps playing.
- For the **recap**: this can take 5–30s. Here we DO show a loading state ("Generating your recap...") because the user has clicked a button and is waiting. `session-status: coaching` event drives this.

---

## 5. Testing strategy

### Rust side

| Test | What it covers |
|---|---|
| `commands::start_session_loads_profile` | `start_practice_session("trumpet")` reads `profiles/trumpet.json`, sets detection range correctly. |
| `commands::end_session_returns_recap_via_mock_generator` | With a mock `RecapGenerator`, full start→phrase→end cycle returns a valid `SessionRecap`. |
| `commands::end_session_handles_generator_failure` | Mock generator returns Err → command returns a minimal local recap, not an error. |
| `commands::double_start_is_rejected` | `start_practice_session` while one is active → error. |
| `state_machine::idle_to_listening_to_idle` | Drives the `AppState` state machine, verifies transitions. |
| `coaching_wiring::phrase_triggers_tip` | Push a phrase into the active session, mock engine returns a tip, verify `coaching-tip` event payload. |
| `coaching_wiring::missing_api_key_disables_coaching_gracefully` | Construct app with `with_env(config, mock, None, None)` → `start_practice_session` succeeds, no `coaching-tip` events, recap returns local fallback. |
| `coaching_wiring::rate_limited_tip_is_dropped` | Two phrases within rate-limit window → only one `coaching-tip` event emitted. |

These go in `apps/desktop/src-tauri/src/commands.rs` (or `tests/` next to it). The existing `CoachingEngine` tests in `crates/brain/src/coaching.rs` are not duplicated — #14 tests wire-up, not the engine's internals.

### Frontend side (Vitest + RTL)

| Test | What it covers |
|---|---|
| `PracticeShell.test` — routes on `screen` enum | Render with `screen: "session"` → `PracticeSession` visible. Same for `"selector"` and `"recap"`. |
| `PracticeSession.test` — Start Practice disabled without instrument | AC: selector-to-session requires an instrument. |
| `PracticeSession.test` — End Session triggers recap | Click end, store status moves to `ending`, mock `invoke("end_practice_session")` resolves, screen moves to `recap`. |
| `SessionTimer.test` — ticks every 1s, stops on `ending` | Fake timers, advance 3s, assert `00:03`. Advance while ending → frozen. |
| `CoachingTipPanel.test` — slides in on new tip | Push tip to store, assert visible, assert aria-live present. |
| `CoachingTipPanel.test` — auto-dismisses after 10s | Fake timers, push tip, advance 10s, assert no longer visible. |
| `CoachingTipPanel.test` — queues multiple tips | Push 3 tips in 1s, assert only first rendered + "2 more" indicator. |
| `SessionRecap.test` — strengths render before areas-to-improve | DOM order check — this is a product invariant, not just styling. |
| `SessionRecap.test` — shows fallback on error | `recapError` set → calm error copy, "Practice again" still works. |
| `practiceStore.test` — state machine rejects invalid transitions | Can't `endSession` from `idle`, can't `startSession` from `listening`. |

### End-to-end / integration

**Smoke test feasibility:**

We CAN write a Rust-side integration test that injects synthetic `AudioEvent`s into the session pipeline (bypassing cpal) and asserts a recap is produced. The hook point: `start_practice_session` can accept an `AudioSource` trait (default = cpal, test = an iterator of pre-baked events). This is ~20 lines of indirection and directly satisfies the final AC: "Integration test: synthetic audio events → phrase detection → mock coaching → UI state updates."

We CANNOT easily write a full Playwright-style end-to-end test that drives the Tauri webview plus real audio. That's a real integration gap — Tauri's webview testing story is immature, and wiring a headless audio device is OS-specific. **My recommendation: punt on webview E2E, invest in the Rust integration test instead.** Frontend integration is covered by component-level Vitest tests with a mocked `invoke`.

### AC → test mapping

| AC from issue #14 | Test(s) |
|---|---|
| Instrument selector shown on launch | `PracticeShell.test` default screen = `selector` |
| Selecting instrument loads profile | `commands::start_session_loads_profile` |
| Start Practice begins capture | `commands::start_session_loads_profile` + `PracticeSession.test` start flow |
| Real-time pitch trace displayed | Existing `PitchDisplay.test`, still passes |
| Coaching tip panel visible | `CoachingTipPanel.test` render |
| Tips slide in, auto-dismiss 10s | `CoachingTipPanel.test` animation + dismiss |
| Session timer visible, MM:SS | `SessionTimer.test` |
| End Session triggers recap | `PracticeSession.test` end flow + `commands::end_session_returns_recap_via_mock_generator` |
| Recap screen shows summary | `SessionRecap.test` render |
| Tauri IPC events `phrase-detected`, `coaching-tip`, `session-recap` | `coaching_wiring::phrase_triggers_tip` + the integration test |
| Zustand store for session state | `practiceStore.test` |
| Free play works without score | Implicit — no score code exists; all tests pass with no score input |
| Integration test synthetic → UI | The `AudioSource` trait integration test |
| `clippy --deny warnings` clean | CI |
| `pnpm lint` + `pnpm test` pass | CI |

---

## 6. Cut lines — what we are NOT doing in this PR

- **OSMD score rendering** — NO. Not in Phase 1.
- **Score following (Matchmaker)** — NO. Story #34.
- **Practice modes** (long-tones, scales, pieces) — NO. Story #21.
- **Real LLM coaching in CI** — NO. Tests use a mock `HttpClient`. Real LLM is opt-in at runtime via env var; CI never hits Anthropic. **Real LLM in local dev** — YES, if the dev has `MUSIC_COMPANION_LLM_API_KEY` set, the real engine is used (nothing new required — `CoachingEngine::new` already handles this).
- **Mobile/tablet layout** — NO. Desktop only. Tailwind responsive classes not added for `sm:`/`md:` on the session view.
- **Teacher dashboard** — NO. Phase 3.
- **Recording / playback** — NO. Explicitly excluded in issue body.
- **Metronome / drone** — NO. Explicitly excluded.
- **Pause/resume** — NO. See §2.
- **Practice history browse** — NO. Story #17. Sessions ARE persisted via existing `SessionStore` (that infrastructure is built), just no UI to view past sessions yet.
- **Cross-session intelligence ("you've struggled with this for 3 sessions")** — NO. Context passed to the LLM is within-session only.

---

## 7. PR slicing

Target: 4 PRs, each <600 lines ideally, <800 max, each testable and mergeable alone.

### PR 0 — Data model: participants + segments (~500 lines)

**Ships:**
- New types in `crates/brain/src/session.rs`: `Participant`, `InputSource`, `InstrumentSegment`, `SegmentId`.
- `SessionRecorder` refactored to operate on `(participants[0].segments[i])` rather than a single flat `phrases: Vec<_>`. Single participant always, but one or more segments over the session's lifetime.
- `SessionRecorder::switch_instrument(&mut self, profile: InstrumentProfile) -> SegmentId` — closes the currently open segment, opens a new one.
- SQLite migration: `participants` and `instrument_segments` tables. Existing `sessions` rows migrate to a single-participant-single-segment shape on first load (idempotent migration).
- `StoredSession` now includes the participants tree.
- Tests: segment transitions, migration idempotency, recap generation across multiple segments.
- **No UI changes.** Entirely in `crates/brain`.

**Merge criterion:** All existing session/store tests still pass post-refactor. New segment tests green. The Ears and Face crates are untouched.

**Why first:** Everything else builds on this. Doing it as PR 0 means PR 1-3 never thrash the data model.

### PR 1 — Scaffolding + timer + mock pipeline (~500 lines)

**Ships:**
- New `practiceStore` with state machine, no coaching.
- `PracticeShell` routing on `screen` enum.
- `PracticeSession` component with `SessionTimer` and `EndSessionButton`.
- Tauri commands `start_practice_session`, `end_practice_session`, `list_instruments` — **with a stub `MockCoachingEngine` and `MockRecapGenerator`** that returns canned responses.
- All IPC events wired.
- Vitest tests for components.
- Rust tests for state machine.

**Behavior:** You can pick an instrument, start a session, see the timer tick, click the instrument name in the header to switch mid-session (calls `switch_instrument`), end, get a recap built from canned data. No real coaching. No real audio pipeline change (Phase 0 `audio-event` still flows to `PitchDisplay`).

**Merge criterion:** AC 1–9 green with mocks. AC 10 (IPC events) green. State machine test green. Segment-switch flow green end-to-end.

### PR 2 — CoachingTipPanel + phrase aggregator integration (~600 lines)

**Ships:**
- `CoachingTipPanel` with slide-in animation, 10s auto-dismiss, queue.
- Phrase aggregator wired to real audio pipeline (`AudioEvent` → `PhraseAggregator::push` on processing thread).
- `phrase-detected` event emission.
- Coaching task spawn on phrase completion (still using `MockCoachingEngine` for deterministic tests in CI).
- Frontend store consumes `phrase-detected` and `coaching-tip`, updates `tipQueue`.
- Rate-limited-tip silent drop.

**Merge criterion:** AC 5–7 (tip panel AC) green. `coaching_wiring::phrase_triggers_tip` green. Integration test (synthetic audio → tip) green.

### PR 3 — Real `CoachingEngine` + recap polish + graceful degradation (~400 lines)

**Ships:**
- Swap `MockCoachingEngine` for real `CoachingEngine`, guarded by env-var detection.
- `SessionRecap` component polished (strengths-first layout, fallback copy, two buttons).
- Missing-API-key graceful degradation path + the capability check command.
- Real `CoachingEngine::generate_recap` wiring (add this method if not already on engine — per architecture doc, recap is a separate LLM call with a different prompt).
- Docs: env-var setup note in README for devs.

**Merge criterion:** AC 8–9 (recap AC) green. Missing-API-key test green. Lint/test/clippy clean.

---

## 8. Open questions for the founder

### Resolved (2026-04-20)

- ~~**Pause vs. one-shot.**~~ → One-shot, wall clock runs through breaks. No pause button in MVP.
- ~~**Mid-session instrument switch.**~~ → **YES, supported.** Drives the `Participant → InstrumentSegment` data model shift (§2.5, new PR 0).
- ~~**Coaching-off default.**~~ → If no API key, show a calm "coaching unavailable" rail. No rule-based filler tips.

### Still open

1. **Coaching on/off: per-session toggle, or global setting?** Assumed **global** (persisted pref, edited from a settings panel we add in PR 3 — or just the rail toggle in the side panel). A per-session toggle adds friction but gives more control. Which?
2. **Tip panel: left or right side?** Picked right. Some users are left-handed / have left-dominant monitor layouts. Worth asking — or do we make it a preference and move on?
3. **"End Session" with zero phrases.** (User clicks Start, never plays a note, clicks End.) Options: (a) show a calm empty-state recap ("Looks like you didn't get to play — come back when you're ready"), (b) skip the recap and return to selector, (c) disable the End button until phrases > 0. I lean (a).
4. **Instrument switch: confirmation modal or instant?** A click on the header instrument name either (a) switches instantly, or (b) pops a small confirm ("Switch to piano? Your trumpet segment will be closed."). (a) is smoother; (b) is safer against accidental clicks during playing. I lean (a) with an undo-like "Switch back" link in the tip panel for 5 seconds.

---

**End of design doc.**
