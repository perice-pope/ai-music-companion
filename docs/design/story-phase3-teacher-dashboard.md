# Story — Teacher Dashboard (Supabase web app): Design Proposal

**Status:** Draft — pending founder + CTO review
**Author:** Design proposal generated for review
**Target story:** *No GitHub issue yet. Suggested title: "Phase 3: Teacher Dashboard — roster, live session feed, assignments, analytics." Suggested labels: `story`, `phase-3`, `cloud`, `privacy`.*
**Phase:** 3, track 2 (recommended second — see `story-phase3-overview.md`).

**Dependencies landed:**
- **Session data model** — `StoredSession` / `SessionSummary` / `PhraseSummary` (`crates/brain/src/store.rs`, `crates/brain/src/phrase.rs`) and the recap. This is *what* gets synced and shown.
- **Recap generation** — the teacher feed is, essentially, students' recaps streamed to their teacher.
- (Recommended) **Tone-quality descriptors** (track 1) on phrases/sessions — makes the dashboard materially richer.

**Dependencies NOT yet built (this story must introduce them):**
- **Accounts / auth** — the desktop app is currently account-less.
- **Cloud sync** — sessions live only in local SQLite today; nothing syncs.

---

## 1. Product framing

### What it is

A **web app** (not embedded in Tauri — teachers open it in any browser) where an instructor follows their students' practice between lessons. Per architecture-v2 §4b: a **student roster**, a **live/recent session feed** (a student finishes a practice session → their teacher sees the recap), **assignments** (assign a piece, set a goal, leave a note), and **progress analytics** (intonation, phrase-quality, tone, and practice-consistency trends per student).

### Why it matters

It turns a solo practice tool into something a **teacher recommends to their whole studio** — the wedge from B2C into B2B/marketplace (architecture's later phases). The student app stays the product; the dashboard is the relationship layer on top.

### The governing constraint: this is a *privacy* project wearing an engineering hat

We're handling **minors' practice data** flowing to a third party (the teacher). FERPA/COPPA and basic trust dominate every decision (architecture-v2 risk table: "High"). Principles:

1. **Student owns their data; sharing is explicit and revocable.** A student (or parent) opts into sharing with a specific teacher. No teacher sees anything without an accepted invite. Unlink at any time.
2. **Row Level Security is the enforcement, not the convenience.** Teachers can read *only* their linked students' data, enforced in Postgres RLS — not in app code that could be bypassed.
3. **The core practice loop never depends on this.** Sync is background and optional; a student with sync off, or offline, practises exactly as today (architecture-v2 §6).
4. **Minimal sync surface.** We sync recap-level summaries (phrase stats, tone descriptors, coaching notes), **not raw audio**, unless separately opted into.

### Things we explicitly reject (this story)

- **Live real-time "watch them practise this second."** Architecture says Realtime subscriptions; v1 of this story syncs **on session completion**, not a live audio/cursor stream. (Live monitoring is a fast follow once the data path is proven.)
- **Teacher ↔ student chat / messaging.** Separate concern.
- **Grades / report cards.** We surface trends and the student's own coaching notes; we don't invent an institutional grading system.
- **A mobile teacher dashboard** — desktop-browser-first (testing-guide §Phase 3 known limitations).

---

## 2. Architecture

```
 Desktop app (student)                 Supabase                      Web app (teacher)
 ┌──────────────────┐         ┌──────────────────────┐         ┌────────────────────┐
 │ SessionStore     │  sync   │ Postgres + RLS       │  read   │ React + TS         │
 │ (local SQLite)   │ ───────►│  profiles            │◄─────── │ roster / feed /    │
 │ recap on finish  │  (opt-in)│ teacher_student_links│  RLS   │ assignments /      │
 │                  │         │  sessions, phrases   │  realtime│ analytics (Recharts)│
 └──────────────────┘         │  assignments         │ ───────►└────────────────────┘
        ▲                     │ Auth (students+teachers)        live feed updates
        │ optional                └──────────────────────┘
   local-first; works fully offline
```

- **Backend: Supabase** (managed Postgres + Auth + Realtime), per architecture-v2 §4b/§6. Off-the-shelf, RLS built in, Realtime for the feed.
- **Teacher frontend: React + TypeScript** (same stack as the student view → component/code sharing; Recharts for analytics, already in the tool table).
- **Sync from desktop: a thin, additive sync module** in the Tauri backend. On session completion (and on a catch-up sweep), push recap-level rows for sync-enabled, teacher-linked students. **One-directional for v1** (student → cloud → teacher); assignments flow teacher → cloud → student app as read-only.

### Data model (sketch — additive, versioned)

```
profiles            (id, role: 'student'|'teacher', display_name, …)         RLS: self
teacher_student_links(teacher_id, student_id, status: 'pending'|'accepted')   RLS: either party
sessions            (id, student_id, instrument, started_at, duration, …)     RLS: owner + linked teacher
session_phrases     (session_id, phrase_index, pitch_stats, dynamics,
                     stability, tone_descriptor jsonb, coaching_note)          RLS: via session
assignments         (id, teacher_id, student_id, score_ref, goal, note,
                     status)                                                   RLS: teacher (write) + student (read)
```

RLS is the spine: every teacher read is constrained to rows where an **accepted** `teacher_student_links` row exists. Students always see their own; teachers never see un-linked or pending students.

### Identity (the cross-cutting decision)

The dashboard forces accounts. We need **one identity** spanning desktop app, dashboard, and (later) mobile. Two shapes (Open Question 2):
- **(A) Fold auth into this story** — the desktop app gains optional Supabase sign-in as part of the dashboard work.
- **(B) A small "Phase 2.5 Sync" story first** — introduce optional accounts + session sync on its own, then the dashboard is purely the *teacher-facing* read layer on an already-synced dataset. Lower-risk slicing; recommended if we want to de-risk auth separately.

---

## 3. Testing & verification

| Test | Covers |
|---|---|
| RLS policy tests (the critical ones) | A teacher **cannot** read an un-linked / pending student's sessions; **can** read an accepted one; a student can't read other students. Tested against a real Postgres (Supabase local / branch DB). |
| Sync module (desktop) | A completed session produces the right recap-level rows; sync-off produces none; offline queues and replays. |
| Link lifecycle | invite → accept → unlink revokes access immediately. |
| Assignment flow | teacher writes → student app reads; student can't write assignments. |
| Analytics aggregation | trend computations match fixture sessions. |
| Frontend (Vitest/RTL) | roster, feed, assignment forms, charts render from mocked Supabase responses. |

**RLS tests are non-negotiable and come first** — a privacy bug here is the worst kind of bug this product can have. Use a Supabase **branch / local stack** in CI so policies are tested against real Postgres, not mocked.

---

## 4. PR slicing

(Assumes Open Question 2 = "Phase 2.5 Sync first." If auth is folded in, PR 1–2 merge.)

### PR 0 (optional, "Phase 2.5") — Optional accounts + session sync (~500 lines)
- Supabase project + `profiles`/`sessions`/`session_phrases` schema + RLS (self-only).
- Desktop: optional sign-in, background sync of completed sessions, fully offline-safe.
- **Merge criterion:** a signed-in student's sessions appear in Supabase; sync-off changes nothing; RLS lets a user read only their own.

### PR 1 — Teacher/student linking + roster (~450 lines)
- `teacher_student_links` + invite/accept/unlink flow + the teacher↔student RLS join.
- Minimal teacher web app shell (auth, roster list).
- Tests: link lifecycle + RLS (teacher sees only accepted students). **The privacy core.**

### PR 2 — Session feed (~400 lines)
- Teacher feed of linked students' recaps; Realtime subscription for new sessions.
- Tests: feed shows only linked students; new session pushes live.

### PR 3 — Assignments (~400 lines)
- Teacher assigns piece/goal/note; student app surfaces assignments (read-only); status round-trip.
- Tests: assignment RLS (teacher writes, student reads), student app rendering.

### PR 4 — Progress analytics (~450 lines)
- Recharts trends per student: intonation, phrase quality, tone trajectory, practice consistency.
- Tests: aggregation correctness on fixtures; charts render.

---

## 5. Cut lines — NOT in this story

- **Live "watching" a student practise in real time** (audio/cursor stream) — feed is session-completion-based first.
- **Messaging / chat, video, scheduling.**
- **Teacher marketplace / curriculum builder** — explicitly a later architecture phase.
- **Institutional grading / LMS integration.**
- **Raw-audio sync** — summaries only unless separately opted in.
- **Mobile teacher dashboard.**

---

## 6. Open questions for the founder

> **Questions 1, 3 (and 4) now have a concrete proposal:** see the companion
> decision doc [`story-phase3-teacher-dashboard-privacy.md`](./story-phase3-teacher-dashboard-privacy.md),
> which is the **gating sign-off** for the teacher-linking work. Questions 2 and 5
> are resolved below.

1. **Privacy/legal sign-off.** FERPA/COPPA + minors' data to a third party (teacher) needs counsel before launch (architecture-v2 risk table). What's the bar for v1 — consult counsel pre-build, or build behind a flag and gate launch on review? **This is the gating question.** → *Proposal: build behind a flag now, gate launch on counsel review of the RLS tests + notice. See privacy doc §1.*
2. **Auth slicing — fold into this story (A) or a separate "Phase 2.5 Sync" first (B)?** (B recommended to de-risk auth/sync independently of the teacher UI.) → **Resolved: (B).** Shipped as #144 (schema) + #145 (optional desktop sign-in + sync).
3. **Who consents for minors?** Student-only opt-in, or parent/guardian consent flow required (likely yes for under-13 per COPPA)? Shapes the onboarding. → *Proposal: parental consent for under-13 (no self-link); student for 13+. See privacy doc §3.*
4. **Hosting the teacher web app** — separate deploy (Vercel/Netlify) vs same infra? New surface area to operate. → *Proposal: separate static deploy. See privacy doc §6.*
5. **Live monitoring priority** — is real-time "watch now" a v1 must, or an accepted fast-follow? (This doc assumes fast-follow.)
6. **Supabase free tier limits** — fine for a pilot; flag the scaling/cost point before a wide launch.

---

**End of design doc.**
