# Story — Mobile (Android / iPhone): Design Proposal

**Status:** Draft — pending founder + CTO review
**Author:** Design proposal generated for review
**Target story:** *No GitHub issue yet. Suggested title: "Phase 3: Mobile — Android + iPhone practice app." Suggested labels: `story`, `phase-3`, `mobile`.*
**Phase:** 3, track 3 (recommended last — see `story-phase3-overview.md`).

**Dependencies landed / referenced:**
- `docs/architecture/mobile.md` — the existing mobile architecture notes (this doc operationalises them into a story).
- **The Rust core (`crates/ears`, `crates/brain`, `crates/transcribe`, and Phase-3 `crates/tone`)** — the entire analysis/coaching/import brain is Rust and, in principle, portable to mobile. The desktop/mobile split is the **Face**, not the Brain.
- **Tauri 2.0** — ships a mobile target (iOS/Android), which is why the desktop stack was chosen; mobile is meant to reuse the Tauri shell + React UI.

---

## 1. Product framing

### What it is

The practice experience — listen, coach, score-follow, recap — on the **phone/tablet** a student already carries. Per `mobile.md` and architecture-v2 §"desktop program (mobile coming in Phase 3)", this is a **port of the Student View**, not a new product. Teacher Dashboard stays web; mobile is student-side.

### Why last

- **Largest scope, least new capability.** It moves an existing, mature experience to a new platform rather than inventing a feature. It benefits from tone (track 1) and optional sync (track 2) already existing, so the mobile app ships the *finished* loop.
- **Real platform friction** lives here: mobile audio capture and latency, touch-first UI, app-store review/policies, background-audio limits, and the **USB-MIDI mess** (`mobile.md` §MIDI: "messy… defer to Phase 3+").

### Honest scoping (from `mobile.md`)

- **Most serious piano students practise at home on a digital piano with USB**, not on a phone. So **MIDI-on-mobile is explicitly low priority** — mic-first. USB-MIDI (iOS Camera Connection Kit / Android USB-OTG) and BT-MIDI are messy and deferred within this story.
- **Mic-based practice (voice, brass, winds, strings) is the mobile sweet spot** — the phone mic is right there.

---

## 2. Architecture

```
        ┌───────────────── shared Rust core (unchanged) ─────────────────┐
        │ crates/ears   crates/brain   crates/transcribe   crates/tone   │
        └───────────────────────────┬───────────────────────────────────┘
                                     │ Tauri 2 mobile (iOS/Android)
                 ┌───────────────────┴───────────────────┐
                 │  React/TS UI (shared, responsive)      │
                 │  + touch-first layouts for small screens│
                 └─────────────────────────────────────────┘
   platform glue (the actual work):
   - mobile audio capture (cpal mobile backends / platform APIs)
   - permissions (mic), background-audio behaviour
   - app lifecycle, store packaging, ONNX Runtime mobile libs
```

The thesis from day one: **the Brain is platform-agnostic Rust; only the Face and the platform glue change.** Mobile's effort is concentrated in:

1. **Audio capture on mobile** — verifying/porting the `ears` capture path to mobile backends (cpal's iOS/Android support, or platform-native capture bridged to the ring buffer). Latency and buffer behaviour differ from desktop and must be re-validated against the latency budget.
2. **ONNX Runtime on mobile** — both `transcribe` (audio import) and `tone` need ONNX Runtime **mobile builds** (iOS static framework, Android AAR/`.so`). This is a new variant of the Phase-2 bundling problem (`runtime.rs` seam), per-mobile-platform.
3. **Touch-first UI** — the React UI is reused, but score rendering (OSMD), the coaching overlay, and recap need responsive/touch layouts for small screens.
4. **Store packaging & policies** — App Store / Play Store review, mic-permission justification, background-audio entitlements.

### Deliberately reused, not rebuilt

- All analysis/coaching/scoring logic (`crates/brain`).
- Pitch/onset/capture *logic* (`crates/ears`) — only the capture *backend* is platform-specific.
- Import + transcription (`crates/transcribe`) and tone (`crates/tone`), given mobile ONNX Runtime.
- The React component library (responsive variants, not rewrites).

---

## 3. Testing & verification

| Test | Covers |
|---|---|
| Core crates on mobile targets | `crates/brain`/`ears`/`tone` compile and unit-test for `aarch64-apple-ios` / `aarch64-linux-android` (logic is platform-agnostic; this proves it). |
| Mobile audio capture smoke | Capture path delivers frames into the ring buffer on a real device/emulator; latency measured against budget. |
| ONNX Runtime mobile load | `transcribe`/`tone` models load via the mobile runtime build (the per-platform analogue of the desktop runtime gate). |
| Responsive UI (Vitest/RTL) | Touch layouts render; existing component tests still pass. |
| Manual device matrix | A short real-device pass per OS (the parts CI can't cover — mic, audio latency, store build). |

CI can cover compilation + logic + UI; **real-device audio latency and store builds need a manual matrix** (like the installer story, some of this is inherently not CI-verifiable from a Linux container).

---

## 4. PR slicing

Target each PR independently mergeable; later PRs need real devices.

### PR 1 — Core crates build for mobile targets (~300 lines, mostly CI/config)
- Add iOS/Android Rust targets; prove `brain`/`ears`/`tone` compile + unit-test there.
- **Merge criterion:** the Brain builds and its tests pass for mobile targets in CI. No app yet.

### PR 2 — Tauri mobile shell + responsive UI (~500 lines)
- `tauri ios init` / `tauri android init`; the React UI runs in the mobile shell with touch-first layouts for the core screens.
- **Merge criterion:** the app launches on simulator/emulator and navigates; no live audio yet.

### PR 3 — Mobile audio capture (~450 lines)
- Wire mobile capture into the `ears` ring buffer; mic permissions; validate latency.
- **Merge criterion:** live pitch/coaching works on a real device; latency within (mobile-adjusted) budget.

### PR 4 — ONNX Runtime mobile + import/tone (~400 lines)
- Mobile ONNX Runtime builds for `transcribe` + `tone`; the per-platform bundling seam.
- **Merge criterion:** audio import and tone assessment work on a real device.

### PR 5 — Store packaging + (optional) sync (~400 lines)
- App Store / Play Store build config, permissions/entitlements; if Teacher Dashboard sync exists, mobile reuses it.
- **Merge criterion:** installable signed builds; (optional) sessions sync.

---

## 5. Cut lines — NOT in this story

- **USB/BT-MIDI on mobile** — explicitly deferred (`mobile.md`); mic-first.
- **Mobile teacher dashboard** — teacher experience stays desktop-web.
- **Mobile-specific new features** — this is a port, not a redesign; net-new mobile-only capability is out.
- **Tablet-optimised "pro" layouts** beyond responsive basics.
- **Offline LLM coaching changes** — same degradation behaviour as desktop.

---

## 6. Open questions for the founder

1. **iOS *and* Android at once, or one first?** (Android is often the cheaper/faster validation; iOS reaches more music students. Picking one halves the platform-glue surface per slice.)
2. **Phone or tablet primary?** Affects UI priorities (score reading is far better on a tablet).
3. **Is mic-only acceptable for v1**, with MIDI explicitly deferred? (This doc assumes yes, per `mobile.md`.)
4. **Developer accounts / signing** — Apple Developer Program + Play Console are prerequisites with their own cost/lead time (and tie into the installer/signing work in issue #129).
5. **Sequencing vs the other tracks** — confirm mobile is genuinely last, or is there a device-first pilot that pulls it forward?

---

**End of design doc.**
