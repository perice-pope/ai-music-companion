# Offline-First & Network Transparency

**Companion doc to:** [`architecture-v2.md`](./architecture-v2.md), [`platform-spine-personalization.md`](./platform-spine-personalization.md), and [`teacher-audit.md`](./teacher-audit.md)
**Status:** Draft
**Date:** 2026-06-10

> **The promise, in plain words:** This app works on a laptop with the Wi-Fi switched off. Everything that matters — listening to you play, telling you how it sounded, and writing up your session afterward — happens *on your device*. The internet is never required to practice. A few extra features (a smarter-sounding coach, syncing your history to the cloud, sharing with a teacher) can use the internet, but each one is something **you** turn on, it starts **off**, and we tell you exactly what leaves your device before it does. Nothing phones home in the background. A parent can hand this to a kid and know that, by default, nothing is being sent anywhere.

---

## The principle

**Offline by default. The internet is NEVER required for core value.** Every networked feature is **opt-in**, **off by default**, and **clearly discloses what leaves the device.**

This is not a nice-to-have. It is a product differentiator and a trust contract with the parents who buy this for their kids. The competitive landscape is full of "sign in to continue" practice apps that treat a child's audio and progress data as the product. We do the opposite: the device is the source of truth, the network is an accessory, and the user is always the one who decides to reach for it.

Two consequences fall out of this and the rest of the doc enforces them:

1. **The full core loop runs with zero network.** Capture → local analysis → recap is all local Rust. If the machine is offline, the only thing that changes is that the LLM narration degrades to its on-device fallback — the loop never breaks and never blocks.
2. **No feature touches the network silently.** Every outbound call is behind a switch the user flipped, and the UI states what is sent and to whom before they flip it.

---

## The core loop is local

The value proposition — *hear me play, tell me how it sounded, write it up* — is entirely on-device.

```text
mic ──► Ears (cpal capture, pitch/onset)            [LOCAL, crates/ears]
          │
          ▼
        Brain (phrase summary, key/intonation/groove/tone,
               MusicalFingerprint, session recap)     [LOCAL, crates/brain]
          │
          ▼
        Face (live meter, tips, recap UI)            [LOCAL, apps/desktop]
```

Every box above is local code with no network dependency:

| Stage | Where it runs | Needs network? |
|---|---|---|
| Mic / MIDI capture | `crates/ears` (cpal) | No |
| Pitch / onset detection | `crates/ears` | No |
| Phrase summary, key, intonation, groove, tone | `crates/brain` | No |
| `MusicalFingerprint` assembly | `crates/brain` (`coaching.rs`) | No |
| Session recap text | `crates/brain` — **offline fallback** when no LLM | No |
| Live UI, tips display, recap screen | `apps/desktop` (React) | No |
| Local session history (SQLite) | Rust core | No |

### The LLM narration degrades, it never blocks — and never fabricates

The one place the core loop *can* reach the network is the coaching narration in
[`crates/brain/src/coaching.rs`](../../crates/brain/src/coaching.rs). It is built so the network is strictly an enhancement:

- `CoachingEngine::get_tip` and `generate_recap` call the LLM through the `HttpClient` trait. On **any** failure — no key, no connection, timeout, unparseable response — they return `fallback_tip()` / `fallback_recap()` instead of an error. The recap is still produced.
- The fallback recap is built **only from locally measured facts** (duration, phrase count, instrument, and the `MusicalFingerprint`). It is generic-but-honest encouragement; it never invents tone, pitch, or key numbers it did not measure. Grounding is preserved: when the engine has no measured key/intonation/groove, those lines are simply omitted (see the `aggregate_*` evidence gates), so the offline path can never assert a fact it didn't observe.
- Result: with the network off, you still get a recap. It reads a little more generic, but every claim in it is true.

This is the technical guarantee behind "the internet is never required for core value."

---

## Every feature that can touch the network

This is the complete enumeration. If a feature is not in this table, it does not make outbound network calls. If a new networked feature is added, it **must** be added here and to the Connections & Privacy surface in the same change.

| Feature | What leaves the device | To whom | Opt-in? | Default | Where it lives |
|---|---|---|---|---|---|
| **AI coaching narration** | Per-phrase analysis numbers and session summary stats (instrument, durations, pitch/tone/intonation/groove figures, prior tips). **No raw audio.** | The configured LLM provider (Anthropic / OpenAI) | Yes | See note¹ | `crates/brain/src/coaching.rs` |
| **Cloud sync (sessions)** | Completed-session recaps: instrument, timestamps, duration, phrase count, overall assessment text, and the session tone aggregate. **No raw audio, no per-phrase rows.** | Supabase project (our hosted Postgres) | Yes | **OFF** (requires sign-in) | `apps/desktop/src/stores/syncStore.ts` |
| **Cloud sync (taste profile)** | Stated personalization preferences (genres, artists, goals, experience). **No raw audio.** Its own switch, independent of session sync. | Supabase project | Yes | **OFF** (separate opt-in) | `apps/desktop/src/stores/syncStore.ts` (`syncTasteProfile`) |
| **Account sign-in / auth** | Email + password (for account creation / login) | Supabase Auth | Yes | **OFF** (no account needed to practice) | `apps/desktop/src/stores/authStore.ts` |
| **Teacher linking / dashboard** | Rides on cloud sync: the same synced recaps become visible to a linked teacher account. | Supabase (teacher-dashboard track) | Yes | **OFF** | builds on sync + auth |

¹ **AI coaching narration default.** The on-device analysis (pitch, key, intonation, groove, tone) and the offline-fallback recap are always available with zero network. The LLM *narration* of that analysis is the networked part. Today the in-app coaching preference (`coachingEnabled`, `practiceStore`) defaults **on**; the principle in this doc says networked enhancements should default **off**. Flipping that default is a behavior change in the Rust/practice layer and is tracked as a **follow-up** below — this doc and the Connections & Privacy surface describe the target state and let the user turn narration off today, at which point coaching is served entirely by the on-device fallback.

### What never leaves the device

- **Raw audio.** No waveform, no recording, and nothing from which audio could be reconstructed is uploaded by any of the features above. (Audio capture for *teacher audit* is a separate, local-only, opt-in feature — see [`teacher-audit.md`](./teacher-audit.md) §Privacy. It writes to local disk and only ever leaves the device through an explicit, in-session "export" action.)
- **The taste profile**, unless cloud sync is on. The personalization profile lives in local SQLite and is only mirrored to Supabase when sync is explicitly enabled (see [`platform-spine-personalization.md`](./platform-spine-personalization.md)).

---

## Disclosure UX

The disclosure surface is **Connections & Privacy**
([`apps/desktop/src/components/ConnectionsPrivacy.tsx`](../../apps/desktop/src/components/ConnectionsPrivacy.tsx)), reachable from the app's
settings/route surface. Its job is to make the principle *visible*, not buried in a policy PDF.

Design rules:

- **One row per networked feature.** Each row states, in plain language: what it sends, to whom, and why you might want it.
- **Each toggle starts OFF** and is the user's switch. Turning one off returns that feature to its on-device behavior (for coaching, the offline fallback).
- **A standing reassurance:** "Everything else works offline — practice, feedback, and your session recap never need the internet." This line is always present, not conditional.
- **"Coach, don't judge" tone.** No dark patterns, no guilt ("you'll lose your data!"), no pre-checked boxes. The copy explains the trade honestly and lets the user choose.
- **No raw-audio claim is made lightly.** Where a feature explicitly does *not* send audio, the row says so, because that is the question parents actually ask.

The existing cloud-sync controls (sign-in / sign-out on the History page) remain functional; Connections & Privacy is the single place that *names every networked feature in one view* and explains it, with the sync opt-in reachable from there.

---

## What we are deliberately NOT doing

- **No telemetry by default.** We do not ship a "usage analytics" pipe that is on out of the box. If we ever add product analytics, it lands in the table above, opt-in, off by default, with disclosure — same rules as everything else.
- **No silent network calls.** There is no background sync, no "check for content updates," no crash-reporter, no font CDN, no analytics beacon firing without a user-flipped switch. The enumeration table is exhaustive by policy: a new outbound call that isn't in it is a bug.
- **No required account.** You can install the app and practice forever without signing in. Sign-in unlocks sync; it is never a gate in front of core value.
- **No "offline mode" as a downgrade.** Offline is not a degraded fallback we tolerate — it is the *default and primary* mode. Networked features are the addition, not the baseline.
- **No raw-audio upload.** None of the networked features above upload audio. (Audio export is local-only and explicit; see teacher-audit.)

---

## Follow-ups (deferred, not in this change)

These require touching the Rust core or sync behavior, which is out of scope for the doc + Face work that establishes the principle:

1. **Default AI coaching narration to OFF (or to a first-run choice).** Change `coachingEnabled` to default off, or add an onboarding step that asks. Backend/practice-layer change in `practiceStore` + any Rust coaching gate. (See note¹.)
2. **A hard "airplane switch" in the Rust core.** A single setting the core respects that prevents the `CoachingEngine` from ever constructing an outbound request, independent of the FE toggle — so the guarantee is enforced below the UI, not just in it.
3. **Build-time / CI assertion of the enumeration.** A test or lint that fails if a new outbound HTTP call site appears in the codebase that isn't represented in the table above and the Connections & Privacy surface.
4. **Disclosure of provider region / data handling** for the LLM narration, once the provider contract is finalized.
