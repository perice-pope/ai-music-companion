# Spec: In-app classroom enrollment + consent flow (#449, enrollment slice)

## 1. Summary

The student-side "Join a classroom" flow (code entry → plain-words consent screen →
`redeem_join_code` → dashboard-sync prompt), current-enrollment display with a
self-serve Leave (revoke), and a minimal teacher-side "My classroom" card (create
classroom, issue/show join code with TTL, roster count) — all on the History page
next to the account panel, with the ConnectionsPrivacy disclosure and offline-first
enumeration row landing in the same PR.

## 2. Problem / why

Migration 0006 shipped classrooms/enrollments, the consent CHECK
(`enrollments_active_requires_consent`) and the two SECURITY DEFINER RPCs
(`issue_join_code`, `redeem_join_code`), and T2 shipped the projection behind
`connectionsStore.dashboardSyncEnabled` — but no student can actually enroll and no
teacher can mint a code. T2 spec §3 explicitly deferred this: "the T-enrollment
slice will prompt for `dashboardSyncEnabled` at classroom join". Issue #449 §2:
enrollment goes `invited → active` **only after the consent screen**, which states
in plain words exactly what the teacher will see; under-13 uses the parental-consent
path (teacher-audit COPPA gate, verbatim).

## 3. Non-goals

- No seat entitlements / commerce wiring (S3), no dashboard web app (T4), no
  teacher-audit audio anything.
- No teacher invite path UI (`enrollments_insert_invite_by_teacher`) — join-code
  redemption is the only student path this slice ships.
- No parent-account model: `consenting_adult_id` stays NULL (0006's recorded
  follow-up). The parent/guardian acknowledgment is an in-app attestation on the
  student's signed-in account, recorded as `consent='parent'`.
- No new migrations, no schema change, no new Rust code, no new network call
  *sites* (all calls ride the existing FE Supabase client).
- No business logic in the FE beyond calling the RPCs: activation, consent
  requirement, under-13 `parent`-only consent, code expiry, revoked-row refusal
  are all enforced by 0006 (CHECK + RLS + the definer functions). The FE mirrors
  them for honest UX; the schema is the truth.

## 4. Contract / interface

### Server contract consumed (0006, verbatim — nothing new)

- `issue_join_code(p_classroom_id uuid, p_ttl interval default '7 days') → text`
  — teacher-only (raises for non-owner), TTL clamped ≤ 30 days, rotates the code.
- `redeem_join_code(p_code text, p_consent text) → uuid` — THE only path to
  `status='active'`. Raises: `'not signed in'`, bad consent value,
  `'invalid or expired join code'`, `'cannot enroll in your own classroom'`,
  `'parental consent required'` (when `profiles.age_tier='under_13'` and
  `p_consent <> 'parent'`), `'enrollment was revoked; ask the teacher to
  re-admit you'`.
- `enrollments_update_student_revoke` policy + guard trigger: a student may
  update their own row only to `status='revoked'` (plus `revoked_at`).
- `classrooms` RLS: owner-only select (row carries the live code + expiry, so
  the TTL display reads the teacher's own row — the server clock's truth).
- Students **cannot** select `classrooms` — so the student-side enrollment list
  cannot show a classroom name. We show the enrollment honestly (status, joined
  date, consent party) and never fabricate a name (silence > lies).
- `profiles` self-select/self-update (0001): the flow reads `role`/`age_tier`
  and, when `age_tier` is NULL, records the user's choice before consent so the
  DB-side under-13 gate actually binds against a real tier.

### New FE surface

```ts
// apps/desktop/src/types/supabase.ts — hand-added (regenerate-to-confirm note):
//   Tables: classrooms, enrollments (column-for-column against 0006)
//   Functions: issue_join_code { p_classroom_id: string; p_ttl?: unknown } → string
//              redeem_join_code { p_code: string; p_consent: string } → string

// apps/desktop/src/stores/enrollmentStore.ts (new; all calls signed-in-gated,
// calm errors — set state, never throw):
loadProfile(userId)            // profiles: role + age_tier (self row)
setAgeTier(userId, tier)       // profiles update; needed BEFORE consent when unknown
loadEnrollments(userId)        // enrollments: own rows (student view)
redeemJoinCode(userId, code, consent /* "student" | "parent" */)
leaveClassroom(userId, enrollmentId)   // update → status 'revoked' + revoked_at
loadClassrooms(userId)         // classrooms (own) + active-enrollment counts
createClassroom(userId, name)
issueJoinCode(userId, classroomId)     // rpc, then re-read the row for code+expiry

// apps/desktop/src/components/ClassroomPanel.tsx (new): the whole flow.
// Signed out ⇒ renders nothing (AccountPanel already offers sign-in).
// Flow steps: idle → code → (age, only if age_tier NULL) → consent → syncPrompt.
```

### Consent screen (the T2 disclosure list, plain words)

Names the **teacher-visible** sync surfaces (datamodel doc §2 P1–P4 + the recap
push, with recap *notes* qualified to the separate 0003 teacher link — a
classroom code alone never grants recap-text access; P5 key mastery is NOT
listed because `learner_model` has no teacher RLS policy and is not
teacher-visible) + the never-list + the joining-alone-shares-nothing note +
revocation immediacy. The bounded claim is scoped to the teacher ("nothing
outside this list is ever shared with your teacher" — other disclosed opt-ins
exist), and the age step is neutral (no consequence hints before the choice). Under-13: states a
parent/guardian must complete the step (a student under 13 cannot self-consent),
requires an explicit guardian acknowledgment checkbox before the accept button
enables, and redeems with `consent='parent'`. 13+ redeems with
`consent='student'`. Exact copy lives in `ClassroomPanel.tsx` and is pinned by
tests.

### ConnectionsPrivacy + enumeration (same PR, standing rule)

- New **InfoRow** (no toggle — every call is user-initiated behind explicit
  buttons + the consent screen; the off-by-default switch count stays 5):
  "Classroom enrollment" — what leaves (join code, consent choice, age group,
  revoke; teacher: classroom name, code mint, roster reads), to whom, when.
- New row in the offline-first enumeration table pointing at
  `enrollmentStore.ts`.

## 5. Acceptance criteria (numbered, testable)

1. **Consent gates the RPC.** Reaching the consent screen makes zero
   `redeem_join_code` calls; the call happens only on explicit accept, with the
   entered code and the correct consent party. Cancelling makes no call, ever.
2. **Under-13 branch.** With `age_tier='under_13'`, the consent screen states a
   parent/guardian must complete it, the accept button stays disabled until the
   guardian acknowledgment is checked, and the redeem goes up with
   `p_consent='parent'`. (13+/adult: single accept, `p_consent='student'`.)
3. **Unknown age.** With `age_tier` NULL, an age step (age group only — never a
   birthdate) runs before consent and persists the choice to `profiles`; picking
   "Under 13" routes into the AC2 branch.
4. **Revoke.** The enrollment list shows current enrollments (status, joined
   date, consent party — no invented classroom names) with a Leave action that,
   after an in-place confirm, updates the student's own row to
   `status='revoked'` (+ `revoked_at`) and refreshes the list.
5. **Teacher card.** With `profiles.role='teacher'`: create-classroom inserts a
   row owned by the caller; each classroom shows its active-roster count and its
   live join code with the expiry read from the row (server TTL, not a client
   guess); "New join code" calls `issue_join_code` for that classroom.
6. **Sync prompt.** After a successful redeem, the prompt appears; accepting
   enables BOTH `cloudSyncEnabled` and `dashboardSyncEnabled` (the dependency
   rule); declining leaves both off and says the teacher will see the student as
   practicing offline — nothing turns on silently.
7. **Calm failures.** A failed RPC/query (e.g. `invalid or expired join code`)
   renders the message calmly in-place, never throws, and leaves the flow
   recoverable. Signed out ⇒ the panel renders nothing and no Supabase call is
   made.
8. **Disclosure present.** ConnectionsPrivacy renders the classroom-enrollment
   disclosure row (join/leave named, user-initiated, sign-in required) with NO
   new toggle — the off-by-default switch-count pin stays 5 — and the
   offline-first enumeration table gains the matching row.

## 6. Edge cases & failure modes

- `age_tier` NULL → AC3 age step; profile write fails → calm error, no consent
  screen (the DB gate must bind before consent is offered).
- Redeem of a revoked enrollment → server raises; we show the message verbatim
  ("ask the teacher to re-admit you") — no client workaround.
- Teacher redeeming their own code → server raises; shown calmly.
- Expired/absent join code on the teacher card → labeled "expired"/"no code
  yet", never a stale code presented as live.
- Multiple enrollments → all listed, each independently leavable.
- Offline / Supabase down → calm error; practice loop untouched (this panel is
  additive UI on History).
- localStorage/profile drift: role is read from `profiles`, not cached.

## 7. Test plan

| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `ClassroomPanel.test.tsx` "the consent screen gates the rpc: no accept, no call" / "cancelling the consent screen never calls redeem" | zero rpc before accept; rpc args on accept; zero on cancel |
| AC2 | "under-13: a parent or guardian must complete consent…" | parent copy present, accept disabled until acknowledgment, `p_consent='parent'` |
| AC2 | "an of-age student consents as themselves" | `p_consent='student'` |
| AC3 | "unknown age: the age step runs first and persists the tier" | profiles update payload; under-13 routing |
| AC4 | "leaving a classroom revokes the student's own enrollment row" | update payload `status='revoked'`, scoped eq id+student |
| AC5 | `enrollmentStore.test.ts` "createClassroom inserts a classroom owned by the caller" / "issueJoinCode mints then re-reads the row" / "loadClassrooms counts only active enrollments"; `ClassroomPanel.test.tsx` "a teacher sees the classroom card…" | insert row, rpc args, count logic, TTL from row |
| AC6 | "the sync prompt enables cloud + dashboard sync only on accept" | both flags true on accept; both false on decline |
| AC7 | "a failed redeem is calm…" + store-level failure tests + "renders nothing signed out" | error text in-place, no throw, zero calls signed out |
| AC8 | `ConnectionsPrivacy.test.tsx` "discloses classroom enrollment without adding a toggle" (+ existing count-5 pin) | info row present, switch count unchanged |

## 8. Architecture / approach

Pure Face-layer slice: one new zustand store owning every enrollment network
call (the same calm-error, signed-in-gated discipline as `syncStore`), one new
component on History, typed against hand-added `types/supabase.ts` rows
(column-for-column against 0006, same regenerate-to-confirm note as T3). The
schema enforces all trust properties; the FE's only "logic" is honest routing
(age step when unknown, parent branch when under-13) — and the server re-checks
both. Offline-first: every call is opt-in by construction (explicit buttons +
consent screen), disclosed in ConnectionsPrivacy + the enumeration table in this
PR. No Rust changes; the disclosure CI scanner (Rust-only) is untouched.

## 9. Slice breakdown

One slice (this PR): types + enrollmentStore + ClassroomPanel + History mount +
ConnectionsPrivacy InfoRow + enumeration row + tests + this spec.

## 10. Risks / open questions

- Students can't see classroom/teacher names (RLS: no student select on
  `classrooms`) — the enrollment list is honest but spartan. A follow-up definer
  function returning `{classroom_name, teacher_display_name}` for own-active
  enrollments would fix it server-side; deferred.
- `consenting_adult_id` remains NULL (no parent accounts) — the guardian
  attestation is app-level, as 0003/0006 comments anticipate. Recorded, not
  hidden.
- `p_ttl` is not surfaced in the UI (server default 7 days, clamp 30) — teacher
  TTL choice is a later nicety.

## 11. References

- Issue #449 §2/§Privacy · `docs/architecture/teacher-dashboard-datamodel.md` §2
- `supabase/migrations/0006_teacher_dashboard_star_schema.sql` (L30–230, L249–387),
  `0007_dashboard_grants.sql`, `0003` (age_tier), `0001` (profiles)
- `docs/specs/449-t2-sync-projection.md` §3/§10 · `docs/architecture/teacher-audit.md`
- `apps/desktop/src/components/{AccountPanel,ConnectionsPrivacy}.tsx`,
  `stores/{connectionsStore,syncStore,authStore}.ts`
