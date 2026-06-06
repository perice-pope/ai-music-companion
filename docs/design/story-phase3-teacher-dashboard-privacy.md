# Story — Teacher Dashboard: Privacy & Consent Model (Decision Doc)

**Status:** Draft — **awaiting founder sign-off. This is the gating document for the teacher-linking work.**
**Author:** Design proposal generated for review
**Companion to:** [`story-phase3-teacher-dashboard.md`](./story-phase3-teacher-dashboard.md) (this resolves its §6 Open Questions 1 & 3, and pins 4)
**Precedent:** [`../architecture/teacher-audit.md`](../architecture/teacher-audit.md) — the established privacy posture for minors' data. This doc reuses its principles and consent component.

> **Not legal advice.** This proposes an engineering + product posture and a set of defaults. The actual launch bar (Q1) still wants a lawyer's eyes — see §1. The point of this doc is to make that conversation concrete and to let us *build the machinery* behind a flag without exposing a single student's data in the meantime.

---

## 0. Why this exists

We've shipped the ungated half of the Teacher Dashboard track:
- **#144** — Supabase schema for a user's *own* data (`profiles`, `sessions`, `session_phrases`), RLS locked to the owner.
- **#145** — optional desktop sign-in + sync of a user's own sessions.

The next tables — `teacher_student_links`, `assignments`, and the teacher-facing **read** policies — are the moment a **minor's practice data becomes visible to a third party (their teacher)**. The teacher-audit RFC names this the highest-stakes class of bug this product can have. We do not write those policies until the questions below have answers signed off. This doc provides recommended answers so the sign-off is a review, not a blank page.

---

## 1. Legal framing (plain English, US-first)

Two regimes apply depending on *who the user is* and *how we reach them*:

| Regime | Triggers when… | What it demands of us (in essence) |
|---|---|---|
| **COPPA** | We knowingly collect personal info from a child **under 13** | **Verifiable parental consent** *before* collection; data minimization; parental rights to review and delete; clear privacy notice. |
| **FERPA** | We operate as a **school's** service handling "education records" | The *school* controls disclosure; we're a "school official" acting under its direction; parents/eligible students get access & amendment rights. Kicks in only on a **school deployment**, not direct-to-consumer. |

Two practical consequences:
1. **Under-13 is the hard line.** Everything below treats an under-13 student as requiring **parental** consent for *any* teacher sharing — stricter than the OS mic permission, exactly as `teacher-audit.md` §Privacy already does for recording.
2. **FERPA is deferred but pre-wired.** We are direct-to-consumer for v1; FERPA doesn't bind us yet. But the consent model below is structured so a school addendum (school-as-consent-authority) slots in without re-architecting. We do **not** pursue school pilots until that addendum exists.

### Q1 recommendation — *how* we clear the legal bar

> **Build the machinery behind a feature flag now; gate the public launch of teacher-linking on counsel review.** Do not block engineering of the RLS/consent plumbing on a legal opinion — block *turning it on for real users* on it.

**Defense:** The RLS tests (the privacy core) are themselves the artifact counsel will want to see — "show me a teacher provably cannot read an un-linked student." Building them first makes the legal review concrete and fast. Shipping them dark costs us nothing and de-risks the timeline.

---

## 2. Scope of shared data — what crosses to a teacher

**Shared (recap-level summaries only):** instrument, session timing & duration, phrase counts, pitch/dynamics/stability stats, **tone descriptors**, the AI's coaching notes and overall assessment, and assignment status. This is the data already in the `sessions` / `session_phrases` schema from #144.

**Never shared by this story:**
- **Raw audio.** Audio sharing is a *separate, separately-consented* path owned by [`teacher-audit.md`](../architecture/teacher-audit.md) (local export, opt-in, its own parental gate). The dashboard syncs **metadata, not waveforms** — same line the architecture draws.
- **Anything from a student who hasn't opted in**, or whose link to that teacher isn't `accepted`.
- **Other students' data** — a teacher's roster is theirs; students never see each other.
- Precise location, device identifiers, contacts, or any field not needed to render a practice trend.

**Principle (inherited from teacher-audit §Privacy):** if it isn't needed to help a teacher coach, it doesn't sync.

---

## 3. The consent model

### 3.1 Roles & age tiers

`profiles.role` is already `student | teacher`. We add an **age tier** to the student side (stored as a coarse flag, **not** a birthdate — data minimization):

| Tier | Who consents to a teacher link | Mechanism |
|---|---|---|
| **Under 13** | **Parent / guardian** (verifiable parental consent) | Adult-account consent dialog, reusing the `consent/` component from teacher-audit §Story D. The student **cannot** self-link. |
| **13–17** | **Student**, with parental visibility | Student accepts the invite; we recommend a parental-notice email where an adult contact exists. |
| **18+** | **Student** | Student accepts the invite directly. |

> We store `age_tier` (an enum), never a date of birth. Knowing "under 13 / 13–17 / adult" is sufficient to gate consent and is far less sensitive to hold.

### 3.2 The handshake (invite → consent → accepted)

```
Teacher                         Supabase (RLS)                 Student / Parent
   │  create invite (email/code) ──►  teacher_student_links            │
   │                                  (status='pending')               │
   │                                                   invite surfaced ►│
   │                                                                    │  consent gate:
   │                                                                    │   • 18+  → student accepts
   │                                                                    │   • 13–17→ student accepts
   │                                                                    │   • <13  → PARENT accepts
   │                              status='accepted'  ◄───────────────── │  (records who consented + when)
   │  can now READ that student's      │
   │  recap rows (RLS join)       ◄────┘
```

- **Default is no access.** A `pending` link grants a teacher **nothing** — RLS only opens on `accepted`.
- **Consent is recorded, not assumed.** The accepting row stores `consented_by` (`student | parent`), `consent_at`, and for under-13 the adult-account id that performed it. This is the audit trail counsel and a parent can both ask for.
- **Sync stays opt-in underneath.** Even with an accepted link, a student with cloud sync turned off shares nothing — there's simply no data in the cloud (the #145 model). Linking and syncing are independent switches; **both** must be on.

### 3.3 Revocation — the part people forget

- **Either party can unlink, any time, one action.** Mirrors teacher-audit's "delete must be one click."
- **Unlink is immediate and total:** the moment `status` leaves `accepted`, the RLS join closes and the teacher can read nothing further — enforced in Postgres, not app code.
- **Already-synced rows:** unlinking **revokes the teacher's *access*** instantly. The student's own rows remain the student's (they still own their history). A teacher retains no private copy — the dashboard reads live through RLS, it doesn't cache student data teacher-side.
- **Right to erasure:** a student deleting a session deletes it for everyone (cascade); a student deleting their *account* cascades all their `sessions`/`session_phrases`/links. We expose a "delete my cloud data" action distinct from "stop syncing."

### Q3 recommendation — *who consents for minors*

> **Parental consent required for under-13 (no student self-link); student consent for 13+ with parental *notice* where we have an adult contact.** Reuse the teacher-audit consent component; do not build a second consent UI.

---

## 4. Data-visibility matrix

What a **linked, accepted** teacher can and cannot see:

| Data | Teacher (accepted link) | Teacher (pending/none) | Other students |
|---|---|---|---|
| Student display name | ✅ | ❌ | ❌ |
| Session recaps (timing, stats, tone, coaching notes) | ✅ read | ❌ | ❌ |
| Per-phrase summaries | ✅ read | ❌ | ❌ |
| Assignments they created | ✅ read/write | ❌ | ❌ |
| **Raw audio** | ❌ (separate opt-in export path) | ❌ | ❌ |
| Birthdate / precise PII | ❌ (we don't store it) | ❌ | ❌ |
| Another teacher's notes on the same student | ❌ | ❌ | ❌ |

A student always sees **all of their own** data and **every assignment** addressed to them (read-only).

---

## 5. RLS enforcement mapping (concrete, against the live schema)

The privacy rules above are only real if Postgres enforces them. Sketch of the policies the linking PR introduces (extends the #144 schema; **RLS tests gate the merge**):

```sql
-- teacher_student_links: either party can see/manage their own links.
create policy link_visible_to_either_party on teacher_student_links
  for select using (auth.uid() = teacher_id or auth.uid() = student_id);
-- (insert by teacher to invite; update to accept restricted to the consenting party;
--  under-13 acceptance additionally requires the actor be the linked adult account.)

-- sessions: owner OR a teacher with an ACCEPTED link to that student.
create policy sessions_visible_to_linked_teacher on sessions
  for select using (
    auth.uid() = student_id
    or exists (
      select 1 from teacher_student_links l
      where l.student_id = sessions.student_id
        and l.teacher_id = auth.uid()
        and l.status = 'accepted'
    )
  );

-- session_phrases: inherit visibility from the owning session (already the #144 pattern).
-- assignments: teacher writes their own; student reads ones addressed to them.
```

The existing self-only `sessions` policy from #144 is **replaced** by the owner-or-linked-teacher policy above — that swap is the single most security-sensitive line in the track and gets dedicated RLS tests (teacher sees accepted ✅, pending ❌, unlinked ❌, after-unlink ❌, other students ❌) run against **real Postgres** (Supabase branch DB in CI), per teacher-dashboard §3.

---

## 6. Operational & security posture

- **Service-role key never ships.** Client uses the publishable/anon key only (the #145 model); RLS is the gate. The service-role key, if ever needed for admin tasks, lives server-side only.
- **Consent audit trail.** `teacher_student_links` retains `consented_by` / `consent_at` / acting adult id — queryable if a parent or auditor asks "who allowed this, when?"
- **Region / residency.** Project is `us-east-2` (#144). Flag for review before any non-US pilot.
- **Breach posture.** Because we sync *summaries, not audio*, the blast radius of a breach is practice metadata — bad, but not a child's voice recording. That asymmetry is deliberate and worth preserving.
- **Q4 (hosting) recommendation:** deploy the teacher web app as a **separate static site** (Vercel/Netlify) talking to Supabase — it shares no surface with the desktop app and can be operated/rotated independently.

---

## 7. Decision checklist (what we need signed off)

Recommended defaults in **bold**; check the box to ratify, or annotate to change.

- [ ] **Q1 — Legal bar:** build linking/RLS behind a flag now; **gate public launch on counsel review** of the RLS tests + privacy notice.
- [ ] **Q3 — Minor consent:** **parental consent for under-13 (no self-link)**; student consent for 13+ with parental notice where possible.
- [ ] Store **coarse `age_tier`, never a birthdate.**
- [ ] **Summaries only** cross to teachers; **raw audio stays on the teacher-audit local-export path.**
- [ ] **Both** "sync on" and "accepted link" required before any datum is teacher-visible.
- [ ] Unlink is **immediate, one action, RLS-enforced**; teacher caches nothing.
- [ ] A distinct **"delete my cloud data"** action separate from "stop syncing."
- [ ] **Q4 — Hosting:** teacher app as a **separate static deploy.**
- [ ] RLS policy tests against **real Postgres** are a **merge-blocking** gate on the linking PR.

---

## 8. What this unblocks

Once the boxes above are ratified, the linking work proceeds exactly as the parent doc's PR slicing (§4), with this doc as the acceptance spec for the privacy-sensitive parts:

- **PR 1** — `teacher_student_links` + invite/accept/unlink + consent gate + the owner-or-linked-teacher RLS swap. **The privacy core; RLS tests first.**
- **PR 2** — teacher session feed (read-only, linked students only).
- **PR 3** — assignments (teacher write / student read).
- **PR 4** — progress analytics.

Until then: nothing is built, and no student's data is reachable by anyone but that student.

---

**End of decision doc.**
