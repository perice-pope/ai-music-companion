# Phase 3 — Design Proposals Overview & Sequencing

**Status:** Draft — pending founder + CTO review
**Author:** Design proposal generated for review
**Scope:** Phase 3 of the architecture (`docs/architecture/architecture-v2.md` §4b, §6; `docs/architecture/mobile.md`). Three tracks, each with its own design doc:

1. [`story-phase3-tone-quality.md`](./story-phase3-tone-quality.md) — the on-device tone-quality model (the proprietary differentiator).
2. [`story-phase3-teacher-dashboard.md`](./story-phase3-teacher-dashboard.md) — the Supabase-backed teacher web app.
3. [`story-phase3-mobile.md`](./story-phase3-mobile.md) — Android / iPhone port.

**What landed before Phase 3 (the foundation these build on):**
- **Phase 1** — Free Play + Score Mode: live capture (`crates/ears`), phrase aggregation (`PhraseSummary`), LLM coaching, session recaps, local persistence (`SessionStore` / `ScoreStore`, SQLite via `rusqlite`).
- **Phase 2 Smart Import** — MIDI + audio import → MusicXML → library, and crucially the **ONNX Runtime sidecar pattern** (`crates/transcribe`, `ort` `load-dynamic`, the bundling seam in `apps/desktop/src-tauri/src/runtime.rs` + `scripts/fetch-onnxruntime.sh`). The tone model reuses this wholesale.

---

## 1. Why these three are one phase — and why they're *not* one PR stream

Phase 3 turns a single-player desktop practice tool into a **platform**: a model that hears *how* you sound (not just whether you hit the note), a way for **teachers** to follow students, and the **devices** students actually carry. They share almost no code, have very different risk profiles, and gate on different things:

| Track | Gated on | Risk shape | Cloud? | Reuses Phase 2? |
|---|---|---|---|---|
| Tone-quality model | Training data | ML accuracy, honesty about limits | No (on-device) | **Yes — ONNX infra** |
| Teacher Dashboard | Cloud sync infra | Privacy/legal (FERPA/COPPA), RLS correctness | Yes (Supabase) | No |
| Mobile | Platform effort | Audio capture parity, store policies | Optional | Indirectly |

Because they're independent, they should ship as **three separate stories**, not interleaved. This doc proposes the order to do them in.

---

## 2. Recommended order

### 1st — **Tone-quality model**

Rationale:
- **Zero external blockers.** Fully on-device, no cloud, no privacy/legal gate. We can start today.
- **Maximum reuse of momentum.** It rides the exact ONNX Runtime infrastructure we just built in Phase 2 (`ort` `load-dynamic`, the bundled-runtime seam). The marginal infra cost is near zero.
- **It's the differentiator.** "No competitor assesses tone quality" (architecture-v2 §5). Shipping it first is the strongest product statement.
- **It enriches everything downstream.** Tone descriptors become a first-class field on phrases/sessions — which makes the Teacher Dashboard materially more valuable (a teacher wants "their tone got airy under the high passages," not just "they played the notes").
- **The honest caveat (training data) is designable.** We don't need a finished model to land the *architecture*: feature extraction, the room-calibration flow, the relative-to-baseline tracking, and a bootstrap/heuristic path are all buildable and testable now, with the learned model dropping into a stable interface later. See that doc's slicing.

### 2nd — **Teacher Dashboard**

Rationale:
- **Depends on cloud sync** (Supabase auth + a sync path from the desktop app), which doesn't exist yet — so it carries the most *new* infrastructure.
- **Benefits from a stable, richer data model.** Doing it after the tone model means the per-session schema it syncs and visualises already includes tone descriptors; we design the sync schema once.
- **Privacy is the dominant constraint**, not engineering. FERPA/COPPA + RLS want careful, unhurried design (architecture-v2 §risk table flags this "High"). Second slot gives that room.

### 3rd — **Mobile**

Rationale:
- **Largest scope, least leverage.** A platform port (mobile audio capture, touch UI, the USB-MIDI mess) that benefits from the desktop feature set being mature first — including tone and (optionally) teacher sync.
- **Naturally last:** it's where the *finished* experience goes to a new device, not where new capability is invented.

> **TL;DR recommended sequence: Tone-quality → Teacher Dashboard → Mobile.**
> Each is independently shippable; nothing forces this order except leverage and risk-staging. The founder can reprioritise (e.g. Teacher Dashboard first if a pilot with a real teacher is the immediate goal).

---

## 3. Cross-cutting concerns (decide once, applies to all three)

- **Account & identity.** Teacher Dashboard introduces real auth (Supabase Auth). The desktop app is currently account-less. We need a single identity model that the desktop app, the dashboard, and eventually mobile all share. *Designed in the Teacher Dashboard doc; flagged here because it's a platform decision, not a dashboard-only one.*
- **Data model as the contract.** `StoredSession` / `PhraseSummary` are the spine all three touch (tone adds fields; dashboard syncs them; mobile produces them). Schema changes should be additive and versioned.
- **"No cloud dependency for the core loop" is sacred** (architecture-v2 §6). Tone runs offline. Mobile's core loop runs offline. Only the Teacher Dashboard *requires* cloud, and only for the teacher-facing layer — the student's practice never blocks on network.
- **Privacy posture is set by the dashboard** but constrains the others (what mobile syncs, what tone recordings we retain for training).

---

## 4. Open questions for the founder (cross-track)

1. **Order — accept Tone → Teacher → Mobile, or reprioritise?** (E.g. is there a specific teacher/pilot driving Teacher Dashboard to the front?)
2. **Is the tone-quality model Phase 2 or Phase 3?** The v2 tool-table tags it Phase 2; v1 calls it a "Phase-3 addition"; we cut it from the Smart Import story. This doc treats it as Phase 3, first slice. Confirm.
3. **Account model timing.** Do we want to introduce optional accounts/sync *before* the Teacher Dashboard (a smaller "Phase 2.5 sync" story), or fold auth+sync into the dashboard story? Affects how much the first dashboard PR carries.
4. **Training-data strategy for tone** (the real gate) — see that doc's Open Questions. Needs a founder/teacher-network answer before the *learned* model slice, though not before the scaffold.

Each track doc below is self-contained with its own framing, architecture, PR slicing, and open questions.
