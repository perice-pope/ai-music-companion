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
| Mic capture | `crates/ears` (cpal) | No |
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

#### The airplane switch is enforced in the Rust core, not just the UI

The engine carries an explicit `NetworkPolicy { Offline | Online }`. It **defaults
to `Offline`**, and both outbound paths (`get_tip`, `generate_recap`) consult it
*first* — before building any prompt, URL, headers, or request body, and before
touching the `HttpClient`. When the policy is `Offline`, there is no code path to
an outbound call: the engine returns the on-device fallback. The command layer
(`apps/desktop/src-tauri/src/commands.rs`) mirrors the user's persisted
`coachingEnabled` preference onto this policy at session start, so the guarantee
lives **below the IPC boundary** — a bug, a malformed IPC payload, or a future
caller that forgets the FE toggle still cannot trigger a silent outbound call.
Tests in `coaching.rs` and `commands.rs` use a mock `HttpClient` that **panics if
hit** to prove that an `Offline` engine never reaches the network.

---

## Every feature that can touch the network

This is the complete enumeration. If a feature is not in this table, it does not make outbound network calls. If a new networked feature is added, it **must** be added here and to the Connections & Privacy surface in the same change.

| Feature | What leaves the device | To whom | Opt-in? | Default | Where it lives |
|---|---|---|---|---|---|
| **AI coaching narration** | Per-phrase analysis numbers and session summary stats (instrument, durations, pitch/tone/intonation/groove figures, prior tips). **No raw audio.** Also the **reveal "why" enrichment** (#253 S2): the detected mode (mode only since #388 — the card no longer claims a key) and the *fixed curated* artist/piece + curated line, so the model only rewords the "why". **No raw audio.** | The configured LLM provider (Anthropic / OpenAI) | Yes | **OFF** (opt-in; see note¹) | `crates/brain/src/coaching.rs` |
| **Cloud sync (sessions)** | Completed-session recaps: instrument, timestamps, duration, phrase count, overall assessment text, and the session tone aggregate. **No raw audio, no per-phrase rows.** | Supabase project (our hosted Postgres) | Yes | **OFF** (requires sign-in) | `apps/desktop/src/stores/syncStore.ts` |
| **Cloud sync (taste profile)** | Stated personalization preferences (genres, artists, goals, experience). **No raw audio.** Its own switch, independent of session sync. | Supabase project | Yes | **OFF** (separate opt-in) | `apps/desktop/src/stores/syncStore.ts` (`syncTasteProfile`) |
| **Cloud sync (learner model)** | Practice-progress blob: reveal collection (concepts + artist names), per-key mastery stats, difficulty step, daily-warmup streak (day count + best score, #257), achieved boss-moment markers (concept + title, best score, achievement count + first-achieved time, #259). **No raw audio, no recordings.** Rides the taste-profile opt-in (progress data, one switch); push-only — local stays authoritative. | Supabase project | Yes | **OFF** (same opt-in as taste profile) | `apps/desktop/src/stores/syncStore.ts` (`syncLearnerModel`) |
| **Cloud sync (teacher-dashboard projection, #449 T2)** | The doc-§2 P1–P4 practice details, on top of the recap push: **session facts** (start/end, wall vs. played seconds, note count, silence ratio, phrase count, instrument, practice mode, app version, the evidence-gated fingerprint, the practised piece's title), **thin phrase rows** (per-phrase start/end seconds, note count, stability, flat tone descriptor, key-estimate name — *no onsets, no pitch curves, no `phrase_json`*), **exercise rows** (label, `spec_hash`, tonic, difficulty, accuracy — *`spec_json` and `seed` NEVER cross; the payload types don't have the fields*), and **tool events** (metronome/band/opener/score-open/narration-used, with their ids-and-numbers-only params). **No raw audio, no note content, no recipes/seeds.** Visible to a teacher only through an ACTIVE classroom enrollment (RLS, migration 0006); revocation closes it on the next query. | Supabase project (star schema, migrations 0006/0007) | Yes — its OWN toggle on top of cloud sync (`connectionsStore.dashboardSyncEnabled`; the enrollment slice will also prompt at classroom join) | **OFF** | `apps/desktop/src/stores/syncStore.ts` (`syncDashboard`) |
| **Account sign-in / auth** | Email + password (for account creation / login) | Supabase Auth | Yes | **OFF** (no account needed to practice) | `apps/desktop/src/stores/authStore.ts` |
| **Classroom enrollment (join / leave / teacher roster, #449)** | Student side: the join code you type, your consent choice (`student`, or `parent` for under-13 — recorded by `redeem_join_code`) and, if not already set, your chosen **age group** (`profiles.age_tier` — never a birthdate); leaving sends the status change to `revoked`, which closes the teacher's view on the next query. Teacher side: the classroom name you create, join-code mint requests (`issue_join_code`), and roster/enrollment reads. **No practice data and no audio — enrollment only links accounts; everything a teacher can *see* is governed by the sync rows above and their own opt-ins.** | Supabase project (`classrooms`/`enrollments` + the two SECURITY DEFINER RPCs, migration 0006) | Yes — every call is user-initiated behind an explicit button, and activation happens only from the in-app consent screen (under-13: parent/guardian acknowledgment required; the server enforces the same gate). Requires sign-in. | **OFF** (no calls unless you act; joining alone shares no practice data) | `apps/desktop/src/stores/enrollmentStore.ts` |
| **Teacher linking / dashboard** | Rides on cloud sync: the same synced recaps become visible to a linked teacher account. For the fuller per-classroom dashboard data, see the **teacher-dashboard projection** row above — a separate, additional opt-in. | Supabase (teacher-dashboard track) | Yes | **OFF** | builds on sync + auth |
| **App auto-update** | A version-check request (current vs. latest), and — only after you confirm in the update dialog — the download of the new signed installer. **No audio, no practice history, no personal data; just the request for the latest version.** | The GitHub release host (`github.com/.../releases/.../latest.json` + the signed installer asset) | Yes — user-initiated (the in-app "Check for updates" button), and the download starts only when the update pill is clicked; #58 adds an OPT-IN "Check for updates automatically" toggle (off by default) that, once enabled, checks on launch + every 4 h and surfaces a pill — download still requires a click | **With the toggle off (default): no network at startup or in the background; a check happens only on explicit user action. With the toggle on: a version-check request only, at launch and every 4 h** | `tauri-plugin-updater` (config: `apps/desktop/src-tauri/tauri.conf.json` → `plugins.updater`; wiring: `apps/desktop/src-tauri/src/main.rs`). See note² |

¹ **AI coaching narration default — now OFF.** The on-device analysis (pitch, key, intonation, groove, tone) and the offline-fallback recap are always available with zero network. The LLM *narration* of that analysis is the networked part. The in-app coaching preference (`coachingEnabled`, `practiceStore`) now defaults **off**: on first run, narration is disabled and the coach is served entirely by the on-device fallback. Turning it on in **Connections & Privacy** mirrors the choice onto the Rust-core `NetworkPolicy` (the airplane switch) and persists it. This was deferred follow-up #1, now implemented.

² **App auto-update — networked, but introduced via a Tauri plugin.** The updater
contacts the GitHub release host to compare the installed version against the
latest release and, *only after you confirm in its dialog*, to download the new
signed installer. Since #58 it is user-initiated OR opt-in-automatic: the
"Check for updates automatically" toggle (`connectionsStore.autoUpdateCheckEnabled`,
off by default, surfaced in `ConnectionsPrivacy`) gates a launch-time + 4-hourly
version check that feeds the bottom-left update pill (`UpdatePill.tsx`); the
download still happens only when the pill is clicked. Without the toggle it
remains wired **user-initiated only** (`plugins.updater.dialog:
true`), **never on startup**, and **never in the background** — so launching the
app and practicing offline make no update request. The egress lives inside the
`tauri-plugin-updater` dependency, not in first-party source, which is why the
disclosure scanner cannot see it (see the registry note below); it is disclosed
here and surfaced in Connections & Privacy as an informational row rather than a
toggle, because the consent gate is the native update dialog rather than a
Face-layer switch. Since #465 the automatic-check toggle has a second consent
surface: a once-only first-launch prompt (`FirstRunUpdatePrompt.tsx`, in the
update pill's slot) that asks the question in plain words and flips the same
`connectionsStore.autoUpdateCheckEnabled` switch on "yes" — the prompt itself
makes no network call, "no thanks" changes nothing, and either answer is final
(the choice remains editable in Connections & Privacy).

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

## Disclosure registry (CI-enforced)

The enumeration table above is not just documentation — it is **enforced in CI**.
A new outbound network call that nobody disclosed cannot land silently.

How it works:

- **The registry.** [`network-call-sites.allowlist`](./network-call-sites.allowlist)
  is a machine-checkable list of every first-party source file allowed to contain
  an outbound-network call site (an `HttpClient` impl, a `reqwest`/`ureq`/`hyper`
  client, or a raw socket egress). Today it contains exactly one entry:
  `crates/brain/src/coaching.rs`.
- **The scanner.** [`scripts/check_network_disclosure.sh`](../../scripts/check_network_disclosure.sh)
  scans the first-party Rust sources (`crates/**`, `apps/desktop/src-tauri/src/**`)
  for outbound-network indicators and **fails** if it finds one in a file that is
  not in the registry. It strips comment-only lines first (so prose that merely
  *mentions* `HttpClient` never trips it) and it cross-checks that every
  registered file is also named in the enumeration table here — defense in depth
  against a registry entry drifting away from the doc.
- **The workflow.** [`.github/workflows/network-disclosure.yml`](../../.github/workflows/network-disclosure.yml)
  runs the scanner on every PR that touches Rust source, the registry, this doc,
  or the scanner itself. It also runs a self-test that injects a throwaway
  undisclosed call site and asserts the checker rejects it, so the guard can't
  silently rot into a no-op.

#### The scanner's blind spot: network introduced via plugins/dependencies

The scanner reads **first-party source only** (`crates/**`,
`apps/desktop/src-tauri/src/**`). Network egress that lives **inside a
dependency or a Tauri plugin** — where the socket is opened by crate code we
don't author — is invisible to it. Such a feature can be fully real and
fully networked while leaving no trace the scanner can match, so it will
**never** appear in `scripts/check_network_disclosure.sh`'s output and cannot be
auto-added to the registry.

Therefore, **network introduced via a plugin/dependency must be added to the
enumeration table above by hand**, in the PR that introduces it. The registry +
scanner remain the automated guard for first-party call sites; this table is the
human-maintained guard for everything else. When in doubt, if a dependency can
reach the network on the user's behalf, it belongs in the table.

**First such entry: the App auto-update feature** (`tauri-plugin-updater`,
landed in PR #174). Its check/download egress is entirely inside the plugin, so
the scanner is blind to it; it is disclosed in the table above (and surfaced in
Connections & Privacy) purely by this hand-maintained discipline. It is **not**
added to `network-call-sites.allowlist`, because that registry is keyed to
first-party files the scanner actually flags — and the updater has none.

### How to add a newly-disclosed call site

A new outbound call is a product decision. To add one, in the same change:

1. Add a row to the **enumeration table** above (feature, what leaves the device,
   to whom, opt-in?, default, where it lives).
2. Surface it in **Connections & Privacy** (`ConnectionsPrivacy.tsx`) — opt-in,
   **OFF by default**, with plain-language disclosure of what is sent and to whom.
3. Make the Rust core honor the **airplane switch** (`NetworkPolicy`) so the call
   cannot fire when the feature is off.
4. Add the file path to [`network-call-sites.allowlist`](./network-call-sites.allowlist),
   with a comment naming the disclosed feature.

If you skip any of these, the `network-disclosure` workflow goes red.

---

## What we are deliberately NOT doing

- **No telemetry by default.** We do not ship a "usage analytics" pipe that is on out of the box. If we ever add product analytics, it lands in the table above, opt-in, off by default, with disclosure — same rules as everything else.
- **No silent network calls.** There is no background sync, no "check for content updates," no crash-reporter, no font CDN, no analytics beacon firing without a user-flipped switch. The enumeration table is exhaustive by policy: a new outbound call that isn't in it is a bug.
- **No required account.** You can install the app and practice forever without signing in. Sign-in unlocks sync; it is never a gate in front of core value.
- **No "offline mode" as a downgrade.** Offline is not a degraded fallback we tolerate — it is the *default and primary* mode. Networked features are the addition, not the baseline.
- **No raw-audio upload.** None of the networked features above upload audio. (Audio export is local-only and explicit; see teacher-audit.)

---

## Follow-ups

### Implemented (the offline-hardening change)

1. **Default AI coaching narration to OFF.** ✅ `coachingEnabled` (`practiceStore`)
   now defaults **off**; on first run the coach uses the on-device fallback.
   Connections & Privacy is where the user opts in, and the choice is persisted.
   (See note¹.)
2. **A hard "airplane switch" in the Rust core.** ✅ `CoachingEngine` carries an
   explicit `NetworkPolicy { Offline | Online }`, defaults `Offline`, and consults
   it before constructing any outbound request — so when offline the `HttpClient`
   is never invoked, independent of the FE toggle. The command layer threads the
   persisted preference into the policy. Proven by tests with a mock client that
   panics if hit. (See *The airplane switch is enforced in the Rust core*.)
3. **CI assertion of the enumeration.** ✅ `scripts/check_network_disclosure.sh`
   + `.github/workflows/network-disclosure.yml` fail the build if an outbound call
   site appears that isn't in the disclosure registry (and the doc table). (See
   *Disclosure registry (CI-enforced)*.)

### Still deferred

4. **Disclosure of provider region / data handling** for the LLM narration, once the provider contract is finalized.
