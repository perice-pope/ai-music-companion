# RFC: Mobile — iOS and Android via Tauri 2

**Status:** Committed direction
**Author:** Perice Pope (+ Claude)
**Last updated:** 2026-04-23
**Related:** [architecture-v2.md](./architecture-v2.md), [eyes.md](./eyes.md)

---

## Why mobile is non-negotiable

The users aren't on desktops. The facts on the ground:

- **Teens live on phones.** A 14-year-old trumpet student will not sit at a laptop to practice. They'll pull out their phone.
- **Schools run iPads.** The common-case fleet device in US music classrooms is an iPad, not a MacBook. We need to ship there or we don't exist for the education segment.
- **Home practice is often a kitchen counter or music stand**, not a desk. A tablet or phone on a stand is the natural form factor.
- **Camera angles are better on mobile.** An iPad on a tripod aimed at a student produces a dramatically better technique-analysis camera angle than a laptop webcam peering up nostrils. See [eyes.md](./eyes.md).

Desktop-first for v1 was correct — it let us dial in the audio pipeline without the latency surprises and permission mazes that mobile adds. But mobile is table stakes for adoption, and the architecture choices we already made were made specifically to enable it.

## Why we're positioned for this

**The Rust core is portable.** `crates/ears` and `crates/brain` are pure Rust (no GUI, no platform APIs baked into the library). They compile to iOS and Android targets today. The only parts that need per-platform adapters are:

- Audio input (cpal abstraction is leaky on mobile)
- Camera input (new, see [eyes.md](./eyes.md))
- MIDI input (mobile MIDI is a niche)
- Storage path resolution (per-platform app data directories)

**Tauri 2 supports iOS and Android.** This is the single biggest reason we chose Tauri over Electron for v1. Electron has no mobile story; Tauri 2 shipped its mobile support in 2024 and it's production-viable for non-trivial apps.

**React frontend ports cleanly.** Touch events, responsive layout, and the existing Tailwind styling work on mobile WebViews (WKWebView on iOS, Android WebView). The UI needs a mobile-first redesign pass, not a rewrite.

## The hard parts

### Audio driver

The single biggest technical risk. `cpal` on mobile is rough:

- **iOS** wants `AVAudioEngine` with proper session configuration. Input latency is device-dependent (AirPods Bluetooth vs. built-in mic vs. USB interface). We'll need a thin `AudioInput` trait with an `AVAudioEngine`-backed impl, probably via `objc2` bindings.
- **Android** wants **Oboe** (which internally picks AAudio or OpenSL ES). Oboe has Rust bindings (`oboe` crate) that work but are not as polished as cpal on desktop.
- **Background audio** on both platforms requires specific entitlements / manifest declarations. iOS will kill audio capture in the background unless we declare the right mode.
- **Latency reality check.** Our desktop budget is <25 ms mic-to-screen. On mobile this may need to become <40 ms for the mid-tier devices we'll actually ship to. That still lets us deliver the "coaching" loop usefully, but per-note visual feedback that feels instant (<20 ms) will be desktop-only.

Plan: define an `AudioInput` trait in `crates/ears`, keep cpal as the desktop impl, add platform-specific impls for iOS/Android behind feature flags. Benchmark end-to-end latency on 3 tiers of device (flagship iPhone, mid-tier Android, iPad Air-class school device) before committing to the mobile perf budget.

### Camera driver (when Eyes lands)

- **iOS** wants `AVCaptureSession` — again, `objc2` bindings.
- **Android** wants **CameraX** (the modern, more sane API) over the raw Camera2.
- Both need proper permission-flow UX, OS-level camera indicator respected, and a kill-switch for parental-consent enforcement.
- Tauri plugins exist for both; we'll likely fork/extend rather than write from scratch.

### Storage

`dirs::data_dir()` gives us per-platform paths on desktop. On mobile, the story is different:

- iOS: sandbox, must use `FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)`.
- Android: scoped storage, `Context.getFilesDir()` for app-private data.
- Tauri 2 provides a unified path API (`@tauri-apps/api/path`) that abstracts both. Use it for all user-visible storage; internal Rust paths already use platform-agnostic `PathBuf`.

### IPC payload size

Desktop IPC was "thin JSON, not heavy serialization" (per CLAUDE.md). Mobile amplifies this: every IPC boundary crossing on mobile is more expensive than on desktop (WebView bridge is slower). Audit the current IPC for payload bloat before the mobile port; strip any "sent every frame" data structures down to essential fields.

## Scope of a mobile v1

**iPad-first for schools** is the right beachhead strategy:

- Schools pay (unlike consumer teens)
- Classroom setting means tripod-mounted iPad, which means reliable camera angle and stable audio
- iPad is a single tightly-controlled device family (no Android fragmentation)
- MDM-friendly deployment is already well-understood in the education market
- Built-in mic quality on iPad Air/Pro is already good enough for pitch detection; USB-C interfaces work for serious students

**Phone second** — iPhone, then Android flagship tier. Phone-form-factor UX is a separate design exercise (the UI is much more constrained than tablet); iPhone ships first because the Apple audio stack is more predictable.

**Deferred:** Chromebooks, Android tablets, web-only. Each has nonzero demand but adds a support surface we can't absorb in v1.

## What mobile v1 ships

Mobile-v1 should be **feature-reduced from desktop**, not feature-mirror. Ship the things that actually work on mobile and defer the things that require a big screen or keyboard:

| Feature                        | Desktop v1 | Mobile v1  | Notes                                              |
|--------------------------------|------------|------------|----------------------------------------------------|
| Real-time pitch display        | ✅         | ✅         | Core value prop                                     |
| Score following (DTW)          | ✅         | ✅         | Requires score rendering — tablet OK, phone tight   |
| Free-play mode (no score)      | ✅         | ✅         | Easiest mode to port first                          |
| Session history                | ✅         | ✅         | SQLite store already portable                       |
| MIDI input                     | ✅         | ⚠️ defer   | USB-MIDI on mobile is messy; BT MIDI is fringe      |
| Camera / Eyes (see eyes.md)    | Phase 2    | Phase 2    | Mobile is *better* camera angle than desktop        |
| Teacher dashboard              | v2         | v2         | Needs a whole-different UX — web app, not mobile    |
| Recording / playback export    | ✅         | ⚠️ defer   | File-sharing flow is different per-platform         |
| Offline-first operation        | ✅         | ✅         | Non-negotiable; schools have flaky networks         |

## Architecture changes

### Crate structure

No crate-level changes. Existing crates compile to mobile targets as-is once audio adapters exist. What we add:

```
crates/ears/
  src/
    audio_input/
      mod.rs        # AudioInput trait
      cpal.rs       # desktop backend (existing)
      avaudio.rs    # iOS backend (new, feature-gated)
      oboe.rs       # Android backend (new, feature-gated)
```

Same pattern for `crates/eyes` when it lands.

### Tauri app structure

```
apps/
  desktop/          # existing — Tauri desktop shell + React
  mobile/           # new — Tauri mobile config + iOS/Android projects
```

Shared React components live in `apps/shared/` and are imported by both. UI-heavy differences (layout breakpoints, touch vs. mouse interactions) live in platform-specific folders.

### Build / release

- **iOS:** `cargo tauri ios build` → `.ipa` → TestFlight → App Store. Requires Apple Developer account ($99/yr), provisioning profiles, code signing.
- **Android:** `cargo tauri android build` → `.aab` → Play Console. Requires Play Developer account ($25 one-time).
- **School deployment:** Apple School Manager (for iPads) and Google Play EDU (for Android tablets). MDM configuration doc required before first pilot.
- **CI:** GitHub Actions macOS runner for iOS, Ubuntu runner for Android. Mobile CI will be slower than desktop CI — accept 15–25 min mobile build time as the floor.

## Latency budget (mobile)

Revised from CLAUDE.md's <25 ms desktop budget:

| Stage                    | Desktop  | Mobile target | Notes                                |
|--------------------------|----------|---------------|--------------------------------------|
| Audio capture            | ~5 ms    | ~10 ms        | OS buffer + mobile audio stack       |
| Pitch detection          | ~6 ms    | ~8 ms         | ARM vs x86, smaller SIMD win         |
| Score alignment          | ~3 ms    | ~4 ms         |                                      |
| IPC + render             | ~5 ms    | ~12 ms        | WebView bridge is the expensive bit  |
| Headroom                 | ~6 ms    | ~6 ms         |                                      |
| **Total**                | **<25 ms** | **<40 ms**  |                                      |

40 ms is still well within the "feels instant" range for coaching cues (human perception tolerates up to ~100 ms for visual feedback before it feels laggy). It's not tight enough for "turn a note red the instant it goes flat" — but that's the Tonestro pattern we're explicitly not doing (v2 §4a/§8).

The CI latency gate on `crates/ears/benches/latency.rs` stays at <25 ms — that's the pure analysis path and platform-agnostic. A **new** mobile end-to-end bench measures IPC + render overhead and gates at <40 ms total.

## Privacy + compliance

Mobile concentrates the privacy risks that already apply to desktop:

- **Microphone permission** on iOS/Android: requires `NSMicrophoneUsageDescription` / `RECORD_AUDIO`. Clear, plain-language rationale required in the permission prompt.
- **Camera permission** (when Eyes lands): same pattern. See [eyes.md](./eyes.md) for the full consent flow.
- **COPPA** for under-13 accounts. Mobile makes this harder because mobile devices are often the kid's primary device — we can't assume the adult is nearby.
- **FERPA** for school deployments. Contract-level protections; separate from the consumer privacy stack.
- **Network transparency.** The app must display clearly when any data leaves the device. Default stance: nothing leaves. Explicit opt-ins for sync, sharing, teacher review.
- **App Store review.** Apple in particular scrutinizes kids-focused apps. Our privacy posture is our App Store defense.

## Sequencing

### Phase 0 — prereqs

- Finish desktop v1 (score following, history UI, coaching layer mature)
- Mobile perf benchmark harness scaffolded (`crates/ears/benches/latency_mobile.rs`)
- Privacy / consent product-design spec written down

### Phase 1 — iPad v0, audio-only

1. `AudioInput` trait + cpal/AVAudioEngine split in `crates/ears`.
2. `apps/mobile/` Tauri project targeting iOS only.
3. Free-play mode (simplest feature) ported and runs on an iPad.
4. End-to-end latency benchmark passes on iPad Air (current gen) and iPad (9th gen, as school floor device).
5. One-week self-dogfooding before opening the repo branch for review.

### Phase 2 — iPad v1, feature parity

1. Score following ported.
2. Session history + storage on iPad.
3. Camera permission scaffolding (even without Eyes yet) so Phase 3 can plug in.
4. First external school pilot — single teacher, ≤10 students, formal feedback loop.

### Phase 3 — Android + iPhone

1. Oboe-backed `AudioInput` impl.
2. Android build target in CI.
3. iPhone UX pass (phone form factor, not tablet).
4. Android tablet support as a follow-up once Android phone is stable.

### Phase 4 — Mobile Eyes

See [eyes.md](./eyes.md). MediaPipe Tasks already runs on mobile; the work is the camera adapter, permission flow, and integration with the existing landmark-heuristic catalog.

## Open questions

- **Bluetooth audio latency.** AirPods add 150+ ms of latency that will swamp any pitch-detection precision. Options: (a) detect BT audio and warn the user, (b) require wired/built-in mic for serious practice, (c) both. Likely answer: both, with a "practice mode" check at session start.
- **MIDI on mobile.** USB-MIDI is possible on iOS via the Camera Connection Kit (yes, really) and on Android via USB OTG, but it's messy. BT MIDI is cleaner but limited. Defer to Phase 3+; most serious piano students practice at home on a digital piano with USB, not on their phone.
- **Offline LLM for coaching notes.** Desktop can run Moondream/Qwen2-VL locally. Mobile can run Moondream on a flagship phone but not a school iPad (9th gen). Options: server-side VLM with explicit parental consent, or feature-degraded mobile coaching (heuristics only, no VLM summaries). Likely answer: degraded mobile v1, server-side opt-in for pilot schools.
- **App Store subscription vs. one-time purchase vs. schools-only licensing.** Business model decision that shapes the mobile build. Out of scope for this RFC.

## Explicit non-goals

- **React Native / Flutter rewrite.** Our Rust core is the asset; we're not throwing it away for UI-framework parity.
- **Progressive Web App.** Mobile browsers can't deliver the audio latency we need. Native only.
- **Windows Phone / KaiOS / other niche mobile.** iOS + Android covers >99% of our target users.
- **Mobile-first teacher dashboard.** Teachers want a big screen and a keyboard. Teacher-facing features live on desktop/web.
