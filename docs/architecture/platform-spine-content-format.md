# Platform Spine — Unified Content / Curriculum Package Format

**Companion doc to:** [`architecture-v2.md`](./architecture-v2.md) and [`platform-modules-addendum.md`](./platform-modules-addendum.md)
**Status:** Draft
**Date:** 2026-06-07

---

## Purpose

This is the **Content Format spine** — one of three shared spines the platform is built on (the other two, written in parallel, are **Commerce / Entitlements** and **Personalization / Cross-Genre**; this doc references them by name and does not redefine them). It defines **a single package format** that serves both the **Audition Simulator** (Module 1: state etude packs + audition rubric) and the **Teacher Curriculum Marketplace** (Module 4: ordered multi-week routines). The whole point: an "audition pack" and a "16 Weeks to All-State" curriculum are the **same structure at different scales** — one ordered sequence of scored steps + coaching rubric + pacing metadata — executed by the **existing Score Mode + LLM Coach** with minimal new code.

---

## Layman's overview

> A practice "pack" is just a teacher's lesson plan turned into a file the app can run: a title, who it's for, and an **ordered list of things to play** — each one is a piece of sheet music plus "here's what to work on" plus "here's how you'll know you've got it."
>
> A one-week audition prep and a four-month curriculum are the same thing, just longer. So we build **one** file format, not two. The app already knows how to show sheet music (Score Mode) and coach you (the LLM Coach) — a pack is simply a playlist that tells those two parts what to do, in what order, and how to grade the vibe.

---

## The format

A **Pack** is metadata + an ordered list of **Steps**. A Step references a MusicXML score, carries practice instructions, a per-step coaching rubric, and completion criteria. That's the entire model. An **audition pack is the 1-unit (or single-section) case**; a **curriculum is the multi-section, multi-week case**. Same schema, same runtime.

Packs are JSON, in the spirit of `profiles/*.json` — data, not code. They reuse the existing vocabulary: **MusicXML** for notation (architecture-v2 §3c — the universal internal format), the **rubric** concept from the LLM coaching engine (the `audition_rubric` prompt variant in addendum Module 1), and **instrument/family** strings that match `profiles/*.json` (`"trumpet"`, `"brass"`, …).

### Schema sketch

```jsonc
{
  "schema_version": 1,                  // see Validation/versioning
  "pack_id": "tx-all-state-trumpet-2026",
  "kind": "curriculum",                 // "audition_pack" | "curriculum" — a HINT, not two runtimes
  "metadata": {
    "title": "Texas All-State Trumpet — 16-Week Prep",
    "author": { "name": "Kris P.", "teacher_id": "tch_0192" },  // resolved by Commerce spine
    "instrument": "trumpet",            // matches profiles/*.json "name"/"family"
    "family": "brass",
    "genre": "classical",               // free-form tag; Personalization spine consumes it
    "rubric": {                         // PACK-LEVEL default rubric (steps may override)
      "criteria": ["tone", "technique", "musicality", "sight_reading"],
      "weights":  { "tone": 0.3, "technique": 0.3, "musicality": 0.3, "sight_reading": 0.1 },
      "coach_prompt_variant": "audition" // selects an existing LLM Coach system-prompt variant
    },
    "pacing": {                         // optional; absent => self-paced, run start-to-finish
      "model": "weekly",               // "weekly" | "sessions" | "self_paced"
      "total_units": 16,
      "recommended_minutes_per_unit": 45
    }
  },
  "sections": [                         // ordered; an audition pack typically has ONE section
    {
      "section_id": "wk01",
      "title": "Week 1 — Lyrical Etude & Long Tones",
      "pacing_unit": 1,                 // maps to pacing.model unit (week 1)
      "steps": [ /* ordered Steps, see below */ ]
    }
    // ... weeks 2..16
  ]
}
```

A **Step**:

```jsonc
{
  "step_id": "wk01-etude-a",
  "title": "Rochut Melodious Etude No. 2 — first half",
  "score_ref": {                        // points at notation; NO notation embedded inline
    "type": "musicxml",                // ONLY musicxml (or .mxl). No proprietary format.
    "uri": "scores/rochut-02.musicxml", // pack-relative path, or a library ScoreId
    "part_index": 0,                   // matches Score Mode's part selection
    "measures": [1, 24]                // optional sub-range; omit => whole part
  },
  "instructions": "Slow, legato. Breathe at the phrase marks, not mid-line.",
  "rubric": {                           // OPTIONAL per-step override of the pack rubric
    "criteria": ["tone", "musicality"],
    "weights": { "tone": 0.6, "musicality": 0.4 },
    "coach_prompt_variant": "audition",
    "focus_note": "Listen for sag on sustained upper-register notes."
  },
  "completion": {                       // how the runtime decides the step is "done"
    "type": "attempts",               // "attempts" | "self_marked" | "coach_signal"
    "min_attempts": 3,
    "advisory_only": true             // "Coach, don't judge": never a hard pass/fail gate
  }
}
```

### How it nests (one format, two scales)

| Concept              | Audition pack (Module 1)                          | Curriculum (Module 4)                                  |
|----------------------|---------------------------------------------------|--------------------------------------------------------|
| `kind`               | `"audition_pack"`                                 | `"curriculum"`                                         |
| `sections`           | usually **1** section (the audition program)      | many sections (weeks)                                  |
| `pacing.model`       | `"self_paced"` (or a short countdown to the date) | `"weekly"` with `total_units`                          |
| `rubric`             | the **audition rubric** (the selling point)       | usually a general-practice rubric, steps may sharpen it |
| Runtime              | **identical** — Score Mode + LLM Coach per step   | **identical**                                          |

An audition pack is just a curriculum with one section and an audition-weighted rubric. We do not build a second model for it.

---

## Authoring & distribution

- **Where packs live.** A pack is a JSON manifest plus its MusicXML files, bundled as a directory or a single `.musapack` (a zip — same trick Score Mode already uses for `.mxl`). On import, scores resolve into the **existing Score Mode library** (the `scores` table / on-disk store from `story-score-mode.md`); the pack manifest is indexed in SQLite alongside it. **One score store, not a parallel one.**
- **How they're loaded.** A new **`crates/brain` loader module** (`brain/src/pack/`) parses and validates the manifest, resolves each `score_ref` to a `ScoreModel` via the existing MusicXML parser, and exposes a `Pack` + ordered `Step` iterator to the runtime. Thin Tauri commands (`import_pack`, `list_packs`, `get_pack`, `advance_step`) mirror the Score Mode command surface. Business logic stays in Rust; IPC is thin JSON (CLAUDE.md).
- **How they're sold.** Deferred to the **Commerce / Entitlements spine** — it owns listings, Stripe/Stripe Connect, the 70/30 teacher split, and gating access to a `pack_id`. This doc only guarantees a pack carries a stable `pack_id` and an `author` reference the Commerce spine can resolve. **We do not define payments or entitlement checks here.**
- **How personalization tailors them.** Deferred to the **Personalization / Cross-Genre spine** — it owns the student taste/musical profile and genre mapping. A pack exposes neutral hooks (`genre`, per-step `focus_note`, the rubric `criteria`) that Personalization reads to re-order optional steps, adjust pacing, or enrich coaching with cross-genre references. **The pack format stays personalization-agnostic; Personalization layers on top.**

---

## How Score Mode + LLM Coach execute a pack

A pack run reuses existing seams; the loader is the only genuinely new piece.

1. **Select a step.** The runtime takes the current `Step` from the loader's ordered iterator (resuming from the last incomplete step persisted in SQLite).
2. **Score Mode renders it.** The step's `score_ref` resolves to a `ScoreModel` + raw MusicXML; this is exactly what `start_practice_session(score_id, part_index)` already consumes (story-score-mode §3). `measures` narrows the cursor range. No new rendering path.
3. **Coach gets the rubric.** The step's `rubric` (or the pack default) selects the LLM Coach **system-prompt variant** (`coach_prompt_variant`) and supplies `criteria`/`weights`/`focus_note` as prompt context — the same mechanism as the addendum's `audition_rubric` field. The Coach still produces whispered tips + a recap; the rubric only **weights what it attends to**. Still "coach, don't judge."
4. **Completion is advisory.** At session/phrase end, the runtime evaluates `completion` (attempt count, self-mark, or a soft coach signal) and **suggests** advancing. Per architecture-v2 §8 ("no auto-grading for auditions"), completion never hard-gates — `advisory_only` is the default and the UI always lets the student move on.
5. **Progress persists.** Per-pack step state (attempts, completed-at, last rubric notes) lands in SQLite next to session history, so a 16-week curriculum resumes across days and a recap can say "Week 3, etude 2 — your phrasing tightened up since Monday."

No new audio path, no new renderer, no new coaching engine. The pack is a playlist over capabilities that already exist.

---

## Validation & versioning

- **`schema_version`** (integer) is required at the top of every pack. The loader rejects unknown major versions with a calm error (same posture as Score Mode's "we couldn't read this score") rather than crashing.
- **Forward-compat:** unknown object fields are **ignored, not rejected**, so newer packs degrade gracefully on older app builds; required fields are minimal (`schema_version`, `pack_id`, `metadata.title`, at least one `section` with one `step`, each step a resolvable `score_ref`). Everything else is optional with sane defaults (absent `pacing` ⇒ self-paced; absent step `rubric` ⇒ inherit pack rubric; absent `completion` ⇒ `self_marked`).
- **Validation surfaces at import**, not at runtime: `import_pack` runs a schema check + verifies every `score_ref` parses as MusicXML, and reports per-step errors so an author can fix a bad pack before it ships. Marketplace upload (Commerce spine) calls the same validator.

---

## Phased delivery (maps to the addendum roadmap)

| Phase | Architecture phase | What ships for this spine |
|---|---|---|
| **Phase 2 — Audition Prep** | Arch Phase 2 (Smart Import + Tone) | `schema_version: 1` format + `crates/brain/pack/` loader + validator. Audition Simulator runs single-section packs through Score Mode + the audition rubric variant. Commerce spine sells them. |
| **Phase 3 — Knowledge + Listening** | Arch Phase 2–3 | Steps gain optional knowledge/reference hooks (consumed by Modules 2/3 and the Personalization spine) — additive fields, no format break. |
| **Phase 4 — Marketplace** | Arch Phase 3 (Teacher Platform) | Multi-section `"curriculum"` packs + pacing + the teacher curriculum builder, which **emits this same format**. Same loader, same runtime — only the authoring UI and Commerce backend are new. |

The addendum already names a "content packaging system (Phase 2)" and a "curriculum package format" for Module 4 as separate line items — **this doc unifies them into one format delivered once in Phase 2 and merely scaled up in Phase 4.**

---

## What we are deliberately NOT building

- **No proprietary notation format.** Notation is **MusicXML only** (`.musicxml` / `.mxl`), referenced, never embedded inline. We do not invent a "Musa score" format; we are not a notation editor (story-score-mode cut line).
- **No second runtime for curricula vs. packs.** One loader, one Score Mode, one Coach. An audition pack and a curriculum differ only in data (section count, rubric, pacing).
- **No payments / entitlements logic in this format.** That is the Commerce spine. A pack carries identifiers; it does not check access.
- **No personalization logic baked into packs.** That is the Personalization spine. Packs expose neutral hooks; they don't re-order themselves.
- **No DRM theater.** Packs are local JSON + MusicXML. We gate *purchase/visibility* (Commerce), not bytes-on-disk; we will not ship fragile client-side encryption that breaks offline practice (the offline core loop is sacred, architecture-v2 §6).
- **No hard pass/fail grading.** `completion` is advisory by default; auto-grading auditions is explicitly out (architecture-v2 §8).
- **No authoring inside the file format spec.** The teacher curriculum *builder* (a Module 4 UI) emits this format; how it's built is a Marketplace concern, not a format concern.

---

## Open questions

1. **`score_ref` indirection:** reference scores by **pack-relative path** (self-contained bundle) or by **library `ScoreId`** (dedup shared etudes across packs)? *Lean: support both — bundled path on import, deduped into the library store.*
2. **Step granularity vs. phrase granularity:** is a "step" always a whole piece/part, or do we need sub-piece "drill these 4 bars" steps as first-class? `measures` covers the common case; do we need richer drill loops in v1?
3. **Completion signals:** how much should `coach_signal` completion lean on the LLM (which can hallucinate) vs. objective measures from `crates/theory` (Phase 4)? Keep advisory until measurement is trustworthy?
4. **Pacing enforcement:** does `"weekly"` pacing ever *lock* future weeks (a structured course feel) or always stay advisory (respect the serious musician)? Architecture-v2's anti-gamification stance pushes toward advisory.
5. **Versioned packs in the wild:** when an author updates a sold curriculum, do students get the new version automatically, pin to the purchased version, or choose? (Touches the Commerce spine — coordinate.)
6. **Cross-spine field ownership:** which fields does the Personalization spine get to *write back* into a running pack instance (e.g. re-ordered optional steps) vs. only read? Define the boundary before Phase 4.

---

**End of design doc.**
