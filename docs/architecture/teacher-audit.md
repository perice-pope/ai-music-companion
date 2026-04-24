# RFC: Teacher Audit — Session Audio Capture for Human Review

**Status:** Exploration → proposed v1 scope
**Author:** Perice Pope (+ Claude)
**Last updated:** 2026-04-24
**Related:** [architecture-v2.md](./architecture-v2.md), [eyes.md](./eyes.md), [sdlc-automation-loop.md](./sdlc-automation-loop.md)

---

## Why Teacher Audit

A parent asked the obvious hostile-user question: what stops a 12-year-old from queueing up Alison Balsom on YouTube, pressing "start session," and cashing in 30 minutes of "practice" they didn't do?

Today's answer is nothing. The ears pipeline sees audio, not a body; a recording of a pro plays through the mic as cleanly as a student does. The student-facing scoring layer is by design **not** an adversary — it coaches tone and pitch, it doesn't run a liveness check.

This RFC proposes the minimum thing that makes cheating meaningfully harder: **capture the session as a WAV and let a trusted adult — teacher or parent — listen back**. Humans are good at "that's not my kid playing" in a second of audio; we don't need to solve detection in code. We just need to make sure a human *can* check.

The same audio archive buys us three other features almost for free:

- **Teacher review** — listen to tone, intonation, and phrasing across a week. Today the recap is metadata-only; a teacher still has to trust our pitch detector's interpretation of a sound they never hear.
- **Parent visibility** — "how's she sounding this week?" without requiring the parent to be in the room.
- **Student self-review** — every serious musician benefits from recording themselves; right now we analyze the audio and then throw it away.

So the primary motivation is anti-cheating, but the feature earns its keep even if nobody ever cheats.

## Non-goals

- **Automated cheat detection.** Classifying "real playing vs. speaker playback" is an open problem; a wrong automated accusation against a kid is worse than no check at all. A human reviewer with context makes the call.
- **Stream-to-teacher-live.** A teacher watching a student practice in real time is a different product (lesson mode). Out of scope.
- **Face recognition, voice biometrics, or any identity verification on audio.** Same reason as eyes.md §non-goals: we're a music coach, not a surveillance product.
- **Cloud-by-default.** Anything that uploads a child's audio without an explicit adult-account action is out of scope for v1. Local-only is the ship target.

## Non-negotiable design constraints

Write these down before engineering or we'll regret it.

1. **Opt-in, never opt-out.** Audio capture starts disabled. Every student (or parent, for under-13) must actively turn it on. No "set up" flow defaults it to enabled. This mirrors the `eyes.md` camera rule — same reasoning, stronger stakes.
2. **Local by default, always.** The WAV lives on the student's disk. No code path uploads audio without an explicit share/export action taken in that same session.
3. **Hardware indicator respected.** We already get the OS mic indicator from cpal; we don't do anything to hide it. If the mic LED is on, it's because the student started a session.
4. **COPPA compliance path.** Under-13 accounts require a parental-consent gate before recording can be turned on, separate from the mic permission the OS asks for.
5. **Delete must be one click.** From the sessions list, "delete recording" removes the WAV immediately. No soft-delete, no 30-day tombstones. If the student wants it gone, it's gone.
6. **Never in the audio thread.** Same rule as the rest of `crates/ears` (CLAUDE.md): zero allocations on the cpal callback. WAV writing happens on a processing thread fed by a ringbuf, exactly like the pitch path.
7. **No new dependency if we don't need one.** `AudioRecorder` already exists in `crates/ears/src/recorder.rs` and writes WAV headers manually. Don't pull in `hound`.

## What already exists

Before proposing anything new, enumerate the bricks already in the repo:

| Module                                    | What it does                                                                                                | Reuse?                                                  |
|-------------------------------------------|-------------------------------------------------------------------------------------------------------------|---------------------------------------------------------|
| `crates/ears/src/recorder.rs` `AudioRecorder` | Opt-in WAV writer (16/24-bit PCM), manual RIFF header, size cap, runs on processing thread                  | **Yes, as-is.** This is the WAV tap.                    |
| `crates/ears/src/retention.rs` `RetentionManager` | Age + size sweep of a WAV dir; `_best` suffix preservation                                                  | **Yes, as-is.** This is the retention engine.           |
| `crates/ears/src/capture.rs` `AudioCapture`   | cpal mic → ringbuf; the producer side of the hot path                                                       | Tee off the consumer thread, as today's pitch path does.|
| `apps/desktop/src-tauri/src/audio_pipeline.rs` | Session-scoped mic → pitch detector → `audio-event` IPC (landed PR #83)                                     | Add a second consumer branch for WAV write.             |
| `crates/brain/src/session.rs` `SessionRecorder` | Accumulates phrase summaries + coaching tips → `SessionRecap`. **Metadata, not audio.**                     | Link to it via `session_id`; do not move audio in here. |

The gap is small and deliberate: the WAV writer, the retention sweeper, and the capture pipeline all exist. What's missing is **wiring**, **a retention default**, **a share/export flow**, and **a teacher persona**. Those are product decisions more than engineering ones, which is why this is an RFC.

## Audio capture — where the WAV is written

### Pipeline shape

Extend `audio_pipeline.rs` with an optional second consumer. The cpal audio thread is unchanged; the processing thread gains a branch:

```text
cpal audio thread ──(ringbuf, lock-free)──► pipeline OS thread
                                                  │
                                                  ├─ PitchDetector::detect()  ──► audio-event IPC
                                                  │
                                                  └─ AudioRecorder::write_samples()  ──► WAV on disk
                                                     (only if recording_enabled)
```

Key properties:

- **One mic, one capture.** We do not open a second input device for recording. The ringbuf consumer already holds every sample; we just give a copy to the recorder.
- **Processing-thread only.** `AudioRecorder::write_samples()` uses `BufWriter<File>` and owns its scratch buffer. Heap traffic is fine there; the audio thread stays allocation-free.
- **Recorder is optional.** `Option<AudioRecorder>` on the pipeline state. Student turns it off → we drop the recorder → next session doesn't write a WAV. No background-on state.
- **Size cap enforced at the WAV level.** `RecorderConfig::max_bytes_per_file` already exists and is u32-clamped to respect the WAV format. A new session cannot silently produce a corrupt file.

### Format and path

- **Format:** 16-bit PCM mono at capture's native sample rate (today 44.1 kHz). Matches what the existing `AudioRecorder` does. We do *not* need 24-bit — this is review audio for humans, not mastering.
- **Path:** `${app_data_dir}/sessions/<session_id>/audio.wav`, where `<session_id>` is the same `SessionId` that `SessionRecorder` already mints. One-to-one correspondence between a recap and its WAV.
- **Sidecar metadata:** `${app_data_dir}/sessions/<session_id>/recap.json` — the existing `SessionRecap` serialized. The export flow (below) reads both.

### Sample-rate reality check

Current capture is 44.1 kHz mono f32 → 16-bit = 88.2 kB/s = **158 MB per 30-minute session**. A kid practicing 30 min/day for a 180-day school year produces **~28 GB** of raw WAV. For two kids sharing a laptop, ~57 GB. That is a lot. See Storage + retention below.

## Storage + retention

The back-of-envelope tells us raw WAV is not the long-term format. Options, in order of increasing complexity:

| Strategy                                        | 30-min session size | Teacher-review quality | Implementation cost        | Notes                                                           |
|-------------------------------------------------|---------------------|------------------------|----------------------------|-----------------------------------------------------------------|
| Keep raw WAV, age-sweep after N days            | 158 MB              | Full                   | **Lowest** — we have it    | `RetentionManager` already does this.                           |
| Compress to Opus at 24 kbps after session ends  | ~5 MB               | Plenty for human ear   | +encoder dependency        | 30× smaller. Opus is free/patent-unencumbered.                  |
| Compress to FLAC after session ends             | ~80 MB              | Lossless               | +encoder dependency        | Half the size; still large. Overkill for review audio.          |
| Stream-encode Opus live (don't write WAV)       | ~5 MB               | Plenty for human ear   | Highest — rework pipeline  | Saves disk writes but complicates the tap. Skip for v1.         |
| Don't store; teacher pulls WAV on demand        | 0 persistent        | Full if caught in time | Low but racy               | Requires the session to *still be on disk* when teacher asks.   |

### Recommendation

**Two-tier retention**, default settings that favor disk over archive:

- **Fresh WAV, 7 days.** Every recorded session writes a raw WAV. `RetentionManager` sweeps WAVs older than 7 days. This is the anti-cheating window: a teacher or parent who has *any* reason to suspect something has a week to listen.
- **Opt-in long-term Opus.** If the student marks a session as "keep for teacher review" (or the teacher does, via export — see below), we transcode to Opus at ~24 kbps and move to `${app_data_dir}/sessions/<session_id>/audio.opus`. The raw WAV is deleted immediately after transcode. Opus files are retained indefinitely, subject to a global size cap.
- **Global cap, `_best` preservation.** Reuse the existing `RetentionManager` size-sweep with `_best` preservation, set the cap to e.g. 2 GB for the Opus archive. Marked-for-teacher files use the `_best` suffix convention so the age sweep never touches them.

### Defense

Why Opus over FLAC or raw WAV? A teacher listening back on laptop speakers is not going to hear the difference between Opus@24 kbps and lossless, and 30× disk wins are the difference between the feature being usable on a Chromebook and being unusable. If we're ever wrong about that, adding a "keep lossless" toggle is a one-line config change.

Why 7 days for the raw window? Short enough that we're not accumulating a month of 158 MB files; long enough that a weekly lesson with a teacher catches the review flow. Tunable in settings, not hardcoded.

Why not "teacher pulls on demand"? Races. The kid ends the session, the retention sweeper runs, file is gone before anyone thinks to check. Retention windows give us a non-racy SLA.

### Implementation note

We need an Opus encoder. Options in preference order:

1. **`opus` crate** (libopus binding) — stable, widely used. Adds a native dep.
2. **`symphonia` + `opus`** — full-featured but heavier than we need.
3. **Shell out to `ffmpeg`** — avoids the dep but introduces an install-path assumption. Reject.

Prefer #1. Transcode happens on a background thread at session-end, not in the live loop, so encoder latency is irrelevant.

## Teacher persona + access model

Today the app has no concept of "teacher" and no account system. Three access models, presented with trade-offs:

### (a) Cloud uploader + accounts

Student signs in, teacher signs in, sessions sync to a server, teacher opens a dashboard.

- **Pro:** The product most parents imagine when they hear "teacher reviews my kid's practice."
- **Con:** Huge scope. Identity, billing, storage costs, residency, SOC 2 eventually, a server. COPPA applies fully once we have accounts for minors — parental consent flow is required by law, not just by taste. FERPA if we sell into schools. This is a 6-month project on its own.
- **Con:** Uploading a child's practice audio to our servers is a privacy decision we should make deliberately, not as a side-effect of shipping a convenience feature.

### (b) Auth + peer-to-peer share

Student and teacher both run the app; pairing code links their installs; sessions replicate directly between devices (e.g. via Syncthing-style sync or a relay we host for NAT traversal only).

- **Pro:** Avoids storing kid audio centrally.
- **Con:** Still requires an identity layer and a relay. P2P is a great story and a hard product. The complexity-to-user-value ratio for v1 is bad.

### (c) Local-only export ("Share with teacher" button)

Student clicks "Export for teacher" on a session. We produce a zip containing `audio.opus` + `recap.json` + a readable `summary.html`. Student sends it via whatever channel they already use — AirDrop to a lesson partner, email to a parent, upload to the teacher's existing LMS (Google Classroom, Canvas, band-teacher-has-a-Dropbox).

- **Pro:** Zero new infrastructure. Zero new privacy surface. Works with whatever sharing setup the family already trusts. Ships in one story.
- **Pro:** Teachers in our target market (band directors, private studio teachers) already have a preferred upload channel. Respecting that is better UX than asking them to learn ours.
- **Con:** No dashboard. Teacher can't "browse all my students' sessions." But that dashboard is v3, not v1 — and if we get there, we build it on top of (c), not instead of it.

### Recommendation

**Ship (c) first.** It's the minimum credible answer to the anti-cheating concern without committing us to identity, cloud, or a server. (b) and (c) are both reachable later from (c)'s data model; (a) is reachable from anywhere but shouldn't be the starting point.

**Explicitly defer (a) and (b).** They go in a follow-up RFC once we have evidence that (c) isn't enough.

## Privacy

This section is non-optional. We're recording audio of a child; the rules are stricter than the COPPA baseline that already applies to the eyes pipeline, because audio is more distinctive than pose landmarks.

### Default settings

- **Recording: OFF.** New install, new student profile, new session: the recording toggle is off.
- **Under-13 accounts: parental-consent gate.** If the profile is flagged under-13, the recording toggle is locked until a parental-consent screen has been acknowledged by an adult-account (same mechanism we already need for eyes.md's camera flow — share the implementation).
- **Data location: local.** `${app_data_dir}/sessions/`. Never written elsewhere without an explicit export action initiated in that session.
- **Telemetry: no audio.** Nothing derived from the audio waveform leaves the device through our existing telemetry. Aggregated scores, yes. Mel-frequency fingerprints, no. If it could be used to reconstruct audio, it stays local.

### Who can see what

- **Student** — owns the files. Can play them back, delete them, export them.
- **Parent (on the same device / filesystem)** — has access by virtue of shared device access. We do not add a second auth layer for parents in v1; a shared family laptop is a shared family laptop.
- **Teacher** — sees only what the student (or parent) exported to them. Teacher never has remote access in v1.
- **Anthropic / us** — never. No audio is uploaded unless we add a cloud feature in a future RFC, at which point it becomes opt-in with its own consent flow.

### Deletion

- **Per-session delete** from the sessions list: removes WAV/Opus + recap.json + any export artifacts still in the cache directory. Immediate, no recycle bin.
- **Nuke all recordings** from settings: single-click wipe of `${app_data_dir}/sessions/*/audio.*`. Recaps (metadata) are preserved unless explicitly also deleted.
- **Uninstall** removes everything; we do not leave the `sessions/` tree orphaned after uninstall.

### COPPA compliance checklist (US)

- [ ] Parental consent obtained before recording can be enabled on an under-13 profile.
- [ ] Privacy policy documents audio capture, retention defaults, and deletion mechanism in plain language.
- [ ] Data minimization: 7-day raw WAV retention, Opus compression for archive, no audio telemetry.
- [ ] Parental right to review: any adult-account profile on the same device can list and play back recordings of a linked child profile.
- [ ] Parental right to delete: same, for deletion.

### FERPA

Pending school-pilot scope. Once we pursue a school deployment, FERPA applies and this RFC needs a schools addendum (same structure as eyes.md's FERPA note). Not blocking v1.

## Anti-cheating effectiveness — an honest assessment

The primary motivation of this feature is to make cheating harder. How much harder?

### The obvious bypass

A motivated 12-year-old holds their phone next to the laptop mic and plays a YouTube recording. Our WAV captures… the YouTube recording. A human reviewer listens back and hears Alison Balsom, notices the suspicious perfection, and the jig is up.

So the audit **works for the unsophisticated case** and fails gracefully for the sophisticated case — the teacher listens, they notice something's off, they ask the kid to play the same passage in the next lesson. The audit buys us the evidence; the lesson is the enforcement.

### This feature does not solve detection

We deliberately don't ship an automated classifier. The reasoning is in non-goals: false positives against kids are worse than false negatives. But we *can* ship a set of **signals a human reviewer can weigh**, without promoting any of them to a verdict:

- **Random audio prompts.** The session randomly asks the student to play a 2-bar fragment ("play a Bb major scale in the next 15 seconds") that wasn't announced ahead of time. A recording can't respond to a prompt issued during the session. If the prompt is present in the WAV and there's a believable response, a reviewer weighs that positively.
- **Baseline comparison.** After a few legitimate sessions, we have a rough tone / pitch-stability / vibrato profile for this specific student. A session that deviates wildly (say, the tone suddenly sounds professional) gets flagged *for a human* to listen to. Never shown to the student. Never used to withhold "credit."
- **Pitch-stability anomalies.** Pro recordings have cleaner pitch curves than a 12-year-old. Our pitch detector already produces the curve. A reviewer can see it alongside the waveform.
- **Ambient-noise fingerprint shift.** A live room sounds different from a compressed YouTube stream played through laptop speakers. Even without training a classifier, spectrogram differences are visible. Again — shown to a reviewer, not auto-judged.

These are **teacher-facing signals, not student-facing judgments**. They live in the teacher-review view, never in the coaching cue layer. Same architectural rule as eyes.md constraint #1: coach, don't judge.

### What we're actually selling

This feature raises the cost of cheating from zero to "I have to hope the teacher doesn't listen." That's a big jump. For the class of student whose parent cared enough to ask about cheating in the first place, it's likely enough. Nobody claims it's an ironclad proof of practice; that's why the recap is called an audit, not an attestation.

## What this changes elsewhere

- **`architecture-v2.md` three-layer diagram** — Ears now has a second persistent output (WAV) alongside the in-flight event stream. Document it in the next v2 revision.
- **`audio_pipeline.rs`** — gains an optional `AudioRecorder` owned by the pipeline thread. Session start/stop wires its `start()`/`stop()` symmetrically with `SessionRecorder`.
- **`SessionRecorder::session_id`** — becomes the authoritative identifier that the WAV path, the recap JSON, and any future export artifact all share. No new IDs minted.
- **`profiles/`** — unaffected. Profile schema does not need to change.
- **Settings store** — new `recording.enabled: bool` (default `false`), `recording.retention_days: u32` (default 7), `recording.keep_for_teacher: bool`, `recording.archive_cap_gb: f32` (default 2).
- **Permissions flow** — `under_13_requires_parental_consent` gate added; share the consent component with eyes.md so we ask once and scope both.
- **Frontend** — new Sessions list shows a "🔴 has recording" indicator, "Export for teacher" action, "Delete recording" action. Playback uses the system audio API, no new player.

## Sequencing

Not now. Land DTW follower (story #34), History UI MVP, and eyes.md Phase 1 first. Teacher audit slots in after those because:

- History UI MVP is the surface teacher audit hangs off of.
- The parental-consent flow eyes.md needs is the same one we need; share it, don't duplicate.
- DTW follower changes what "phrase" means in the recap, which the teacher export will render. No point designing the export against a recap shape that's about to change.

## Rough implementation plan

Four stories, each with acceptance criteria. Scope only — no code yet.

### Story A — Wire `AudioRecorder` into the session pipeline

- **Goal:** Every session can optionally produce a WAV at `${app_data_dir}/sessions/<session_id>/audio.wav`.
- **Scope:** Tap the existing `audio_pipeline.rs` ringbuf consumer; construct an `AudioRecorder` from the existing `RecorderConfig` when `recording.enabled` is true; call `write_samples()` alongside `PitchDetector::detect()`; call `stop()` on session end.
- **Acceptance criteria:**
  - [ ] With `recording.enabled: false`, no WAV file is written. Existing pitch-detection behavior is unchanged.
  - [ ] With `recording.enabled: true`, a valid WAV (`ffprobe` or `hound` round-trip reads it cleanly) exists at the expected path after session end.
  - [ ] The cpal audio-thread callback contains no allocation added by this story (verified by the existing `audio_thread_output_test` pattern).
  - [ ] `max_bytes_per_file` cap kicks in cleanly on a synthetic long session — the file stops growing at the cap and an event is logged.
  - [ ] Dropping the pipeline mid-session flushes and closes the WAV header correctly (no truncated/invalid file).

### Story B — Retention defaults + Opus archive path

- **Goal:** Disk usage stays bounded; marked sessions survive retention.
- **Scope:** Schedule `RetentionManager::sweep` on app start and on session end with `max_age_days: 7`; add "keep for teacher" flag on a session that renames the WAV with `_best` suffix; introduce Opus encoder on a background task; transcode marked sessions to Opus and delete the raw WAV on success.
- **Acceptance criteria:**
  - [ ] Unmarked WAVs older than 7 days are deleted on sweep; marked ones (`_best` suffix) are preserved.
  - [ ] Marking a session as "keep" produces `audio.opus` within 10 s (for a 30-min session) on a reference laptop.
  - [ ] Raw WAV is deleted only after Opus transcode succeeds; transcode failure leaves the WAV intact.
  - [ ] Size sweep caps the Opus archive at the configured limit and surfaces `bytes_over_quota_after_sweep` if best-takes alone exceed the cap.
  - [ ] "Nuke all recordings" from settings removes every file under `sessions/*/audio.*` and leaves recap JSON intact.

### Story C — Local export flow ("Share with teacher")

- **Goal:** Student can produce a single artifact to hand to their teacher.
- **Scope:** Sessions list gains "Export for teacher" action; produces `${session_id}-export.zip` containing `audio.opus` (or `audio.wav` if Opus isn't ready yet), `recap.json`, `summary.html`. Zip is written to a user-chosen folder via the system file-picker. No upload.
- **Acceptance criteria:**
  - [ ] Export produces a zip whose contents open cleanly in standard tools on macOS, Windows, and Linux (manually verified on each).
  - [ ] `summary.html` renders standalone in a browser with no network requests — all CSS inline, no external fonts.
  - [ ] Exporting a session without a recording produces a zip with recap + summary only, no audio; the summary notes that recording was off.
  - [ ] Export does not mutate the original session — recap, WAV, and retention state are unchanged after export.
  - [ ] User-facing string surface is localized through the existing i18n layer (no hardcoded English in the export button, toast, or summary template).

### Story D — Teacher persona, parental consent, privacy defaults

- **Goal:** Recording is genuinely opt-in, with an adult-account gate for under-13 profiles.
- **Scope:** Add `is_under_13: bool` to the profile schema; add `parental_consent_recording: Option<DateTime>` field that's set only via an adult-account dialog; gate the recording toggle on it; document all defaults in the privacy policy.
- **Acceptance criteria:**
  - [ ] A fresh install of the app has `recording.enabled: false` with no override possible until the toggle is consciously flipped.
  - [ ] On an under-13 profile, the toggle is disabled in UI until `parental_consent_recording` is set. Unit-tested at the settings-store layer.
  - [ ] The parental-consent dialog is the same component that eyes.md uses (consolidated into a `consent/` module), not a duplicated implementation.
  - [ ] Privacy policy doc (`docs/privacy/recording.md`, new) describes: what is captured, where it is stored, 7-day default retention, how to delete, what is never uploaded.
  - [ ] An end-to-end test verifies: toggle off → start session → no WAV. Toggle on (adult profile) → start session → WAV exists.

## Open questions

- **What does "keep for teacher" mean in the UI?** A star icon on a session? A dedicated "good take" button mid-session? A bulk "mark this whole week" from a history view? The retention engine supports all three — the UI choice is a product decision. Prefer the mid-session "good take" button; it's the moment the student knows, and it matches how serious musicians already self-archive.
- **Should the coaching layer know about recordings?** Right now it doesn't — recording is a side channel. But there's a coaching move like "that was a great take; want to save it for your teacher?" that could live in the cue stream. Defer until Story A lands and we see how the UI feels.
- **Windowed recording vs. whole-session.** Today's proposal writes the entire session. An alternative is a 30-second ring buffer of the most recent audio, where a "save that" click promotes the buffer to disk — same pattern as eyes.md's rolling frames. Trade-off: whole-session is simpler but wasteful; ring-buffered is stingier but means an unprompted teacher review has nothing to review. Whole-session with aggressive retention (7 days) resolves the tension; revisit if disk pressure becomes a user complaint.
- **Cross-device teacher access (a and b).** Punted to a follow-up RFC, but worth pre-wiring: the export zip format in Story C is the data contract. Whatever cloud or P2P story we tell later consumes that zip; the student-facing export flow is unchanged.
- **Watermarking student audio with a session nonce.** A small inaudible tone at a known frequency, mixed in at session start, gives a reviewer a weak "yes this came out of our app" signal. Interesting, but adds complexity for defenders more than it slows attackers. Park it.

## Explicit non-goals (restated, since they tend to drift)

- Automated cheat classification. A human reviewer makes the judgment, always.
- Real-time upload to a teacher during a session. Live lesson mode is a separate product.
- Cloud storage of any audio in v1. Ever, until a follow-up RFC says otherwise.
- Face / voice biometric identity verification. We don't need to prove *who* it is; we need a human to be able to tell.
- Storing audio fingerprints or derived audio embeddings in telemetry. Nothing that could reconstruct the sound leaves the device.
