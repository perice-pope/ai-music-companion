# Teacher Dashboard — Data Model (BI Spec)

**Companion doc to:** [`platform-spine-commerce.md`](./platform-spine-commerce.md),
[`teacher-audit.md`](./teacher-audit.md), and
[`offline-first-and-network-transparency.md`](./offline-first-and-network-transparency.md)
**Status:** Draft (design; precedes any build — issue #449, goal G1)
**Date:** 2026-07-19

---

## Purpose

The founder mandate, verbatim: *"the lifeblood of what we are selling is VISIBILITY INTO THE
PRACTICE SESSION data."* This document is the BI-grade data model behind that sentence: what a
band director can see, exactly which rows produce it, and how the numbers stay honest. It covers
five layers, device to dashboard:

1. **Local telemetry additions** — what the desktop app must start recording (audited against what it records today).
2. **The sync projection** — which rows and aggregates go up, and the privacy contract around them.
3. **The cloud star schema** — facts and dimensions a dashboard queries.
4. **Dashboard v1 views** — the exact queries and the questions each answers.
5. **The fudge-vector table** — every way a kid fakes practice, and which metric catches it.

Founder decisions folded in (issue #449 comment, 2026-07-19): pricing is **per-classroom
per-year**; **no parent view** for now; remaining open questions resolved **best-call** and
recorded at the end of this doc.

---

## Layman's overview

> A band director with 60 kids can't listen to 60 practice sessions a week. What she *can* do is
> glance at a screen that answers four questions honestly: **What did each kid practice?** (which
> cells, which keys, which pieces). **Is it getting better?** (accuracy over time, per key).
> **How are they practicing?** (with the metronome? with the band? at what tempo?). And the one
> nobody else sells: **did practice actually happen?** — not "the app was open for 30 minutes"
> but "the horn made sound for 4 of those 30, here's the gap." The app already refuses to flatter
> a thin session to the student's face (#445-6b: a 10-minute session with 60 seconds of playing
> is called "a quick touch," on the played-time clock, never the wall clock). This doc extends
> that same honesty to the teacher's screen — and prices it, because that honesty *is* the
> product.

---

## What the local store knows today (the audit)

Everything below already exists in the device's SQLite (`crates/brain/src/store.rs`) or the
recap JSON (`crates/brain/src/session.rs`). The dashboard is mostly a *projection* problem, not
a *collection* problem — with one gap, itemized after the table.

### Already captured

| Question | Where it lives today | Grain |
|---|---|---|
| WHAT was practiced (generated material) | `exercise_log` — `source`, `label`, `spec_json` (full replayable `VariationSpec`), `seed`, `difficulty`, `tonic`, `accuracy` | one row per exercise the engine generated |
| WHAT was practiced (scores) | `sessions.score_id` → `scores` (title, composer); per-phrase `score_span` + `verdicts` (hit/near/missed); recap `score_summary` | per session / per phrase |
| WHAT it sounded like | recap_json `fingerprint` (`MusicalFingerprint`: tone, key + `key_claim`, intonation, groove) — each dimension evidence-gated, `None` over a lie | per session |
| HOW it progressed | `learner_model.key_mastery` — per key/scale `Mastery { attempts, accuracy_ewma, owned, last_epoch_secs }` (EWMA, not sticky: slipping loses the key); `exercise_log.accuracy` over time per `spec_json`/`tonic` | rolling / per exercise |
| HOW long, really | `session_phrases` — per phrase `start_secs`, `end_secs`, `note_count`, `phrase_json` (full `PhraseSummary`: pitch stats, stability, tone, key, onsets). Summed phrase time is the **played clock** #451 already quotes | per phrase |
| Session frame | `sessions` — `started_at`, `ended_at`, `duration_secs` (wall), `phrase_count`, `instrument`, `practice_mode`, `app_version`, `recap_json` | per session |
| Saved openers | `starter_recipes` (name, items, direction) | per recipe |
| Stated identity | `taste_profile` (genres, artists, goals, experience, is_under_13) | per user |

### Tool usage: logged vs. not logged (the gap)

The founder wants "HOW they practiced" — tool usage. Audit result:

| Tool | Logged today? | Evidence |
|---|---|---|
| **Openers** | ✅ Yes | `exercise_log` `source = "opener"` (Begin logs exactly one row; graded on completion) |
| **Score practice** | ✅ Yes | `exercise_log` `source = "score_practice"` + phrase `verdicts` + recap `score_summary` |
| **Explore / lessons / lifts / bridges** | ✅ Yes | `exercise_log` sources `explore`, `explore_chip`, `lesson`, `lift`, `measure_bridge`, `jam_bridge`, `progression_lift` |
| **The Pocket (metronome): on/off, mode (anchor/follow/handoff), tempo** | ❌ **No** | Tempo persists only in `localStorage` (`practiceStore.ts` `POCKET_TEMPO_KEY`); mode + playing state are ephemeral Zustand. No record survives the session. |
| **Band ("Play with me" accompaniment): on/off, key pin** | ❌ **No** | `accompanimentPlaying` / `keyPinned` are ephemeral store state fed by `accompaniment-status` events. Nothing persisted. |
| **Coaching narration on/off during the session** | ❌ No (preference only) | `coachingEnabled` is a persisted *preference*, not a per-session fact. |

So: **the material trail is already excellent; the tool trail is half missing.** A teacher can
already learn *what* a student drilled and how it scored, but not whether they used the click,
at what tempo, or whether the band carried them. That gap is the one new local table below.

---

## 1. Local telemetry additions

### 1a. `practice_events` — the tool-usage journal (NEW)

One new append-only table in the local SQLite, written best-effort (like
`log_exercise_best_effort` — a telemetry failure must never break practice), never updated,
never read on the hot path. **No audio, no pitch data, no content** — only which tool, when,
with what knobs.

```sql
-- HOW the student practiced: tool usage during a session. Append-only,
-- event-sourced, one clock (seconds from session start — the same clock
-- #451's played-time copy uses). Local-first like everything; leaves the
-- device only under the enrollment sync opt-in, disclosed in
-- ConnectionsPrivacy BEFORE it can sync.
CREATE TABLE IF NOT EXISTS practice_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,          -- SessionId of the containing session
    at_secs REAL NOT NULL,             -- seconds from session start (one clock)
    kind TEXT NOT NULL,                -- event vocabulary below
    params_json TEXT NOT NULL DEFAULT '{}'  -- kind-specific knobs; additive
);
CREATE INDEX IF NOT EXISTS idx_practice_events_session
    ON practice_events(session_id, at_secs);
```

**Event vocabulary (v1)** — deliberately small; `params_json` is additive so new knobs never
need a migration:

| `kind` | `params_json` | Emitted when |
|---|---|---|
| `pocket_start` | `{"bpm": 90, "mode": "anchor", "count_in": true}` | click starts |
| `pocket_stop` | `{"bpm": 96}` | click stops (final tempo) |
| `pocket_mode` | `{"mode": "follow"}` | anchor/follow/handoff change |
| `pocket_tempo` | `{"bpm": 104}` | tempo change while running (coalesce: ≤1 row per settled value) |
| `band_start` | `{"key_pinned": false}` | accompaniment locks on and plays |
| `band_stop` | `{}` | accompaniment stops |
| `band_key_pin` | `{"tonic": 7, "mode": "dorian"}` | user pins the band's key |
| `opener_begin` | `{"recipe": "named-or-null"}` | opener panel Begin |
| `score_open` | `{"score_id": "…"}` | a score is loaded for practice |
| `narration_used` | `{"kind": "tip" \| "recap"}` | an LLM narration actually fired (already opt-in) |

Rules, inherited from the house style:
- **Append-only.** Corrections are new events, never UPDATEs. Same discipline as `exercise_log`.
- **One clock.** `at_secs` is offset from `sessions.started_at` — joins trivially against
  `session_phrases.start_secs/end_secs`, so "was the click running while she played phrase 7?"
  is a range join, not a timezone puzzle.
- **Never in the audio thread.** Events are emitted from the command layer / pipeline OS thread,
  exactly where `log_exercise_best_effort` already lives.
- **Disclosure before sync.** The table is local-only until the enrollment sync ships; the same
  PR that syncs it adds the row to the offline-first enumeration table and to
  `ConnectionsPrivacy.tsx` (the standing rule: a networked surface in neither is a bug).

### 1b. Session integrity aggregates (NEW columns, derived at session close)

The anti-fudge numbers must be **computed once, in Rust, at session save** — not re-derived in
three dashboards that drift apart. Additive columns on `sessions` (via
`add_column_if_missing`, the shipped migration idiom):

```sql
ALTER TABLE sessions ADD COLUMN played_secs REAL;      -- Σ phrase (end_secs − start_secs): the #451 clock, persisted
ALTER TABLE sessions ADD COLUMN note_count INTEGER;    -- Σ phrase note_count: voiced events actually detected
ALTER TABLE sessions ADD COLUMN silence_ratio REAL;    -- 1 − played_secs / max(duration_secs, ε), clamped to [0,1]
```

These are the same quantities the #445-6b thin-session recap already computes at narration time
("about 60 seconds of actual playing"); persisting them makes the honesty queryable instead of
recomputed. **Precedent, cited:** #451 pinned the rules — *one clock* (copy quotes summed
played time, never the wall clock), *judged score sessions are never thin* (follower accuracy
has its own denominator), *no self-contradiction*. The dashboard inherits all three verbatim.

### 1c. Exercise retry key (NEW column on `exercise_log`)

Retries of the same material are an anti-fudge signal *and* a legitimate practice signal — the
dashboard needs to count them cheaply without parsing `spec_json` per query:

```sql
ALTER TABLE exercise_log ADD COLUMN spec_hash TEXT;    -- FNV-1a 64 of spec_json (the score_content_hash idiom), tonic excluded
```

Same-cell-different-key rows share a `spec_hash` and differ in `tonic` — which is exactly the RV
question ("did she row this cell through the keys, or grind one key?") expressed as GROUP BY.

That is the **entire** local footprint: one new table, four additive columns. No new IDs — every
row keys off the `SessionId` that `SessionRecorder` already mints.

---

## 2. The sync projection (device → cloud)

The device SQLite stays the source of truth; sync remains a **one-way, append-only projection
up**, exactly like today's session push (`syncStore.ts`). The cloud never writes back into the
local DB. Everything below rides the **enrollment sync opt-in** (issue #449 §3): off by default,
enabled per device, consent screen states in plain words what a teacher will see, revocation
closes visibility instantly via RLS.

| # | What goes up | Grain | Notes |
|---|---|---|---|
| P1 | `sessions` row (existing push) **+ new fields**: `played_secs`, `note_count`, `silence_ratio`, `practice_mode`, `score_id`→title, `app_version` | per session | fingerprint JSONB already flows |
| P2 | `session_phrases` — thin rows: `phrase_index`, `start_secs`, `end_secs`, `note_count`, `stability`, `tone` (flat descriptor), key estimate name | per phrase | the cloud table exists (migration 0001) but the client doesn't push it yet; this projection activates it. **Not** the full `phrase_json` — no onsets vector, no pitch curves |
| P3 | `exercise_log` rows: `logged_at`, `source`, `label`, `spec_hash`, `difficulty`, `tonic`, `accuracy` | per exercise | `label` is the human name F1 generated ("Minor triad, enclosed, descending") — that's the teacher-facing material name. **`spec_json` and `seed` stay local**: replayability is a device concern, not a dashboard one |
| P4 | `practice_events` rows: `at_secs`, `kind`, `params_json` | per event | small by construction (tempo coalescing); no content |
| P5 | `learner_model.key_mastery` (existing learner-model push) | rolling | already disclosed; the dashboard reads the EWMA snapshot |

**Privacy notes (the contract, restated where it can't be missed):**

- **No raw audio, ever, on this path.** Nothing in P1–P5 contains or can reconstruct audio.
  Teacher-audit audio remains a separate, local-only, explicit-export feature
  ([`teacher-audit.md`](./teacher-audit.md) §Privacy) — it is *not* part of the dashboard sync.
- **Disclosure first.** P2–P4 are new sync surfaces: each lands with its row in the
  offline-first enumeration table + `ConnectionsPrivacy.tsx` in the same PR, opt-in, OFF by
  default. The consent screen a student (or parent, under-13 — COPPA gate from teacher-audit,
  verbatim) sees at enrollment names all five surfaces in plain words.
- **Honest absence.** A student enrolled without sync shows as "practicing offline" on the
  dashboard — a labeled state, never zeros (issue #449 §3). Absence of data is displayed as
  absence, not as failure; same "silence > lies" rule the recap follows.
- **Aggregates are computed on device where they gate honesty** (`played_secs`,
  `silence_ratio`) and **in the warehouse where they are pure rollups** (weekly heat). The rule:
  anything that *judges* a session is computed once, in Rust; anything that *sums* sessions is
  SQL.

---

## 3. The cloud star schema

Supabase/Postgres, same conventions as everything shipped (uuid PKs, RLS on, `(select
auth.uid())` policies, teacher access via the `teacher_student_links` / `enrollments`
active-link join from migration 0003 and issue #449 §2). Facts are append-only and arrive via
the client projection under the student's own insert policy; teachers get SELECT-through-link
policies only. BI naming is explicit so dashboard SQL reads like the question it answers.

### Dimensions

```
dim_student      = public.profiles (existing)                    -- who
dim_classroom    = classrooms + enrollments (issue #449 §2)      -- which roster, seat, consent state
dim_date         = calendar table (school-year aware: year_label,
                   week_of_year, is_school_day)                  -- when, in band-director units
dim_material     (material_id uuid pk,
                  spec_hash text unique nullable,                -- generated material (cells/patterns)
                  score_id uuid nullable → scores mirror,        -- pieces
                  label text not null,                           -- "Minor triad, enclosed, descending" / "Haydn Concerto, mvt 1"
                  source text not null,                          -- opener|lesson|explore|…|score_practice
                  kind text not null)                            -- 'cell' | 'score'
                  -- upserted lazily from P3 rows; the 12 keys of one cell
                  -- are ONE material row (tonic lives on the fact — the RV
                  -- unit of practice is the cell, not the key)
```

### Facts

```
fact_session     (session_id uuid pk,                            -- P1, grain: one session
                  student_id → dim_student, started_at, ended_at,
                  duration_secs, played_secs, note_count,
                  silence_ratio, phrase_count, instrument,
                  practice_mode, score_material_id nullable,
                  fingerprint jsonb, app_version)

fact_phrase      (session_id, phrase_index, pk(session_id, phrase_index),  -- P2
                  start_secs, end_secs, note_count, stability, tone jsonb,
                  key_name text nullable)

fact_exercise    (exercise_id uuid pk,                           -- P3, grain: one generated exercise
                  student_id, session_id nullable,               -- session linkage when known
                  logged_at, material_id → dim_material,
                  tonic smallint, difficulty smallint,
                  accuracy real nullable)                        -- NULL = generated but never graded (kept: abandonment is a signal)

fact_tool_event  (event_id uuid pk,                              -- P4, grain: one tool event
                  session_id, student_id, at_secs, kind, params jsonb)
```

### RLS posture (reuses the shipped idioms)

```sql
-- Students write/read their own facts (the sessions_insert_own idiom).
-- Teachers read through an ACTIVE enrollment only — the migration-0003
-- accepted-link join, keyed on enrollments.status = 'active':
create policy fact_session_select_teacher on fact_session for select using (
  exists (select 1 from enrollments e
          join classrooms c on c.id = e.classroom_id
          where e.student_id = fact_session.student_id
            and c.teacher_id = (select auth.uid())
            and e.status = 'active'));
-- Same policy shape on fact_phrase / fact_exercise / fact_tool_event
-- (join through the session's student). Revocation either direction
-- closes the view on the next query — no cleanup job in the trust path.
```

### Rollups (materialized views, refreshed on a schedule — never trusted for consent)

```sql
-- The workhorse: one row per student per calendar day.
create materialized view mv_student_day as
select s.student_id, d.date_key,
       count(*)                              as sessions,
       sum(s.duration_secs)                  as wall_secs,
       sum(s.played_secs)                    as played_secs,
       sum(s.note_count)                     as notes,
       avg(s.silence_ratio)                  as avg_silence_ratio,
       count(*) filter (where s.played_secs < 20 or s.phrase_count < 3)
                                             as thin_sessions   -- the #451 thresholds, verbatim
from fact_session s join dim_date d on d.date_key = s.started_at::date
group by 1, 2;

-- Material × key coverage: the RV question as a table.
create materialized view mv_material_key as
select e.student_id, e.material_id, e.tonic,
       count(*)                              as attempts,
       count(accuracy)                       as graded,
       avg(accuracy)                         as avg_accuracy,
       max(logged_at)                        as last_at
from fact_exercise e group by 1, 2, 3;
```

---

## 4. Dashboard v1 views

Scope decision (recorded): **v1 = roster heat + per-student drill-down + engagement-integrity
panel** (material coverage rides inside drill-down). Assignment-push is deferred (see decisions
at the end). Each view below states the question it answers and the query that answers it.

### 4a. Roster heat (the landing view)

**Question:** *"Across my classroom, who practiced this week — really?"*
One row per student, one cell per school day, colored by **played minutes** (never wall
minutes), with the integrity flag baked into the cell.

```sql
select p.display_name, d.date_key,
       round(m.played_secs / 60)                        as played_min,
       round(m.wall_secs / 60)                          as wall_min,
       m.thin_sessions > 0 or m.avg_silence_ratio > 0.8 as integrity_flag
from mv_student_day m
join enrollments e on e.student_id = m.student_id and e.status = 'active'
join dim_student p on p.id = m.student_id
where e.classroom_id = $1 and d.date_key >= $week_start;
```

Cell renders `played_min`; hovering shows `wall_min` next to it — the gap *is* the story. A
student with no synced rows renders "practicing offline," never a zero.

### 4b. Per-student drill-down

**Question:** *"What is this kid working on, and is it getting better?"* Three panels:

- **Session timeline** — `fact_session` ordered by `started_at`: played vs wall bar pairs,
  phrase count, instrument, mode, fingerprint chips (tone/key/groove when measured; blank when
  the gate failed — the dashboard never out-claims the recap).
- **Material progress** — accuracy over time per cell and per key:

```sql
select m.label, e.tonic, date_trunc('week', e.logged_at) as wk,
       avg(e.accuracy) as acc, count(*) as attempts
from fact_exercise e join dim_material m using (material_id)
where e.student_id = $1 and e.accuracy is not null
group by 1, 2, 3 order by wk;
```

  overlaid with the `learner_model.key_mastery` EWMA snapshot (`accuracy_ewma`, `owned`,
  `attempts`) as the "current standing" column — the same non-sticky honesty the wheel shows the
  student.
- **Material × 12-key coverage matrix** — `mv_material_key` pivoted: rows = cells (one
  `dim_material` row each), columns = 12 tonics, cell = `avg_accuracy` (blank = never drilled).
  This is RV philosophy as a report: the cell is the unit; the matrix shows whether it was rowed
  through the keys or camped in one.

### 4c. Material coverage (classroom rollup)

**Question:** *"Which cells/keys has the class collectively covered — and what has nobody
touched?"* `mv_material_key` aggregated over the roster; sorts by least-covered. Feeds the
director's next-rehearsal choice; becomes the seed of assignment-push later.

### 4d. Engagement-integrity panel

**Question:** *"Which practice claims should I not take at face value?"* One row per flagged
session, most recent first:

```sql
select p.display_name, s.started_at,
       s.duration_secs / 60           as wall_min,
       s.played_secs / 60             as played_min,
       s.silence_ratio, s.note_count, s.phrase_count,
       (select count(*) from fact_exercise e
        where e.session_id = s.session_id and e.accuracy is not null) as graded,
       (select max(cnt) from (select count(*) as cnt from fact_exercise e
        where e.session_id = s.session_id
        group by e.material_id, e.tonic) r)                           as max_retries
from fact_session s join dim_student p on p.id = s.student_id
where s.silence_ratio > 0.8
   or (s.duration_secs > 600 and s.played_secs < 120)
   or s.phrase_count < 3
order by s.started_at desc;
```

Tone rule, inherited from "coach, don't judge": the panel **surfaces evidence, it never issues
verdicts**. Copy is "45 min open, 3 min of sound" — a fact the teacher interprets — never
"cheating detected." Automated accusation of a kid is a non-goal here for exactly the reasons
teacher-audit §non-goals gives; the human makes the call, the data just makes the call easy.

---

## 5. The fudge-vector table

Every entry is a way a student can inflate apparent practice, mapped to the row-level evidence
and the metric that exposes it. This table is the acceptance-test spec for the integrity panel:
each vector becomes a fixture session that must trip its named metric.

| # | Fudge vector (how a kid fakes it) | What the data actually records | Metric that catches it |
|---|---|---|---|
| F1 | Start a session, walk away for 30 min | wall 30 min; ~0 phrases; ~0 notes | `played_secs` ≈ 0, `silence_ratio` ≈ 1 → thin-session flag (#445-6b thresholds: < 3 phrases or < 20 s played) |
| F2 | Leave the TV / talking / room noise on so "something" is heard | few sparse phrases; low `note_count`; low `stability`; fingerprint gates mostly `None` (no key, no groove) | notes-per-played-minute floor + empty fingerprint on a "long" session |
| F3 | Noodle aimlessly instead of assigned material | phrases exist, but `fact_exercise` is empty / ungraded for the period | graded-exercise count = 0 while `played_secs` high; coverage matrix stays blank |
| F4 | Re-grade the same easy exercise to farm accuracy | many `fact_exercise` rows sharing one `(material_id, tonic)` | `max_retries` per session; distinct-material count ≪ attempt count |
| F5 | Camp in one comfortable key instead of rowing the cell | attempts concentrate in one `tonic` for a `spec_hash` | 12-key coverage matrix: 1 hot column, 11 blank |
| F6 | Split practice into many 90-second sessions to farm streaks/counts | high `sessions`, tiny median `played_secs` per session | `mv_student_day`: sessions vs played_secs ratio; thin_sessions count |
| F7 | Play a recording of a pro into the mic (the Alison Balsom problem) | clean phrases, *implausibly* clean: accuracy/stability jump far above the student's `key_mastery` EWMA baseline | baseline-deviation surface (flag-for-human, per teacher-audit §signals) → teacher-audit audio listen is the actual check; metrics only nominate |
| F8 | Run the metronome/band loudly and let the app "hear practice" | `fact_tool_event` shows pocket/band running; note detection during those spans is thin (the pipeline gates the click out of pitch) | tool-on spans with near-zero phrase `note_count` inside them (range join `fact_tool_event` × `fact_phrase`) |
| F9 | Practice with sync off all term, enable it before grading | server-side `created_at` ≫ session `started_at` for a burst of rows | late-arrival annotation on the heat map ("synced Jun 3, played May 1–30") — shown, not punished |
| F10 | Doctor the local DB / clock (the determined liar) | self-reported rows are self-reported | out of metric scope by design — this is what teacher-audit's human listening exists for; the dashboard raises cost, the lesson is the enforcement |

The honest line we sell (same one teacher-audit draws): metrics move cheating from "free" to
"requires deliberate effort that a listening teacher will still catch." F1–F6 and F8 are caught
cold by the schema above; F7 and F9 are nominated to a human; F10 is explicitly not a metrics
problem.

---

## Decisions recorded (formerly open questions)

Best-call rulings per the founder's 2026-07-19 note; each is a doc-level decision that any
future PR may revisit *by editing this section*, not by silently diverging.

| Question | Decision | Rationale |
|---|---|---|
| Teacher-license price shape | **Per-classroom per-year** (founder decision) | matches how band programs budget; maps to one `b2b_seatpack` purchase row per classroom-year in the commerce spine |
| Parent view | **None for now** (founder decision) | teacher + student surfaces only; parent visibility rides the shared-device reality (teacher-audit §who-can-see-what) |
| Dashboard v1 scope | **Roster heat + per-student drill-down + engagement-integrity panel**; material coverage ships inside drill-down | this is the sellable core of "visibility"; nothing here blocks on content tooling |
| Assignment-push (push a recipe/curriculum to the class) | **Deferred to v2** | needs the content-format spine's assignment shapes; coverage view (4c) gives the director the manual workflow meanwhile |
| Free-tier caps | **3 imported scores, 5 saved starter recipes**; local core loop, 12-key rows, recaps, and full local history stay free forever (the #449 degradation matrix is unchanged) | numbers picked to be generous enough to evaluate, small enough to convert; enforced as entitlement gates, not data deletion |
| Retention for lapsed cloud data | **12 months** | a lapsed license keeps its cloud rows for a school year, so renewal restores the dashboard exactly; local data is never touched (offline-first) |

---

## What we are deliberately NOT building

- **No automated cheat verdicts.** The integrity panel surfaces evidence; the human judges.
  (Teacher-audit non-goal, inherited verbatim — a false accusation against a kid is worse than a
  missed fake.)
- **No audio on the dashboard path.** Audio stays local-only + explicit-export
  (teacher-audit); nothing in P1–P5 can reconstruct sound.
- **No keystroke/screen surveillance, no liveness checks, no camera.** We measure the music that
  reached the mic, nothing else.
- **No second telemetry pipe.** `practice_events` is the one tool-usage journal; product
  analytics do not piggyback on it, and nothing in it syncs outside the enrollment opt-in.
- **No dashboard writes into the practice loop.** The cloud never writes back to the device;
  the projection is one-way by construction (no merge problem, no remote control of a kid's app).
- **No gamification of the integrity metrics.** Streak-farming (F6) exists because streaks are
  incentives; we report time-and-truth, we don't mint badges for it.

---

## Sequencing (slices, the house loop)

1. **T1** — local: `practice_events` table + emitters (pocket, band, opener, score_open) + the
   three `sessions` integrity columns + `spec_hash`. Backend-only; tests per event kind and per
   aggregate (fixtures from the fudge table F1/F4/F5/F8).
2. **T2** — projection P1–P4 + ConnectionsPrivacy rows + offline-first enumeration table
   entries (same PR, per the standing rule).
3. **T3** — cloud schema migration (facts, dims, RLS through active enrollment) + rollup views.
4. **T4** — dashboard v1 (web): roster heat → drill-down → integrity panel, in that order.

Each slice is independently shippable; T1 is useful even if nothing ever syncs (the student's
own History page can show played-vs-wall honestly, which is the same honesty resold upward).
