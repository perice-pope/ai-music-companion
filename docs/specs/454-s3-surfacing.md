# Spec: method-book tips reach both founder surfaces (#454 S3)

## 1. Summary
S2 shipped the evidence-gated selection engine and a thin `method_book_tip`
command; nothing surfaces yet. S3 wires the tip into both surfaces the issue
names: the **post-session recap** (offline generator appends one attributed
line; the LLM generator receives the tip as GROUNDED INPUT) and the **#453
coaching box** (the box learns to speak a second voice, with a visible
attribution line). Attribution is non-negotiable everywhere — #454's
copyright posture is attributed paraphrase, and the attribution is what makes
the paraphrase legally safe and honest.

## 2. Contract

### 2.1 Recap (live end-session path)
- `RecapInput.method_book_tip: Option<pedagogy::PedagogyEntry>`
  (`serde(default)` — stored inputs load unchanged). Threaded exactly like
  `history_suggestions`: a new `generate_recap_with_context` parameter; the
  recorder stays pedagogy-agnostic (`to_recap_input*` sets `None`).
- **Resolution seam:** `end_practice_session_impl` resolves the tip at the
  point it already resolves the family — `select_pedagogy(&family,
  &coaching::build_fingerprint(&completed.all_phrases()))`. This is the live
  path: THIS session's phrases, never a store read-back (the S2 command's
  `list_recent` path is the *between-sessions* read; using it here would race
  the save and speak to the previous session). `build_fingerprint` goes
  `pub`: it is the same per-phrase aggregation the generators run over the
  same phrases — an O(phrases) summary pass, cheap enough that one extra
  computation at the command layer beats plumbing a fingerprint through the
  generator contract. The evidence gates inside it are reused, not copied.
- `PedagogyEntry::source_line()` (`"{author}, {title}"`) becomes the single
  attribution formatter — the S2 DTO and both recap paths call it, so the
  copy can never drift from the command's.
- **Offline generator** (`grounded_offline_recap`, which also backs
  `fallback_recap`): when a tip exists, append ONE line —
  `"{guidance} ({source_line})"` — to **`areas_to_improve`**, at the tail,
  the same structural slot as the #453 history append (after the measured
  lines and the empty-filler decision). The guidance ships verbatim (it is
  already the founder's voice: "There are drills for exactly this in
  Schlossberg's …"); the parenthesized `source_line` guarantees the formal
  attribution is in the copy for every entry, whatever its phrasing.
- **Why `areas_to_improve` is the honest home:** the tip exists *only*
  because THIS session's measured fingerprint crossed a deficit bar (S2 is
  evidence-gated end to end) — it is a deepened diagnosis of a measured
  area, "here is what the method books say about the thing I just measured".
  `next_session_suggestions` is the history voice's home (#453 appends
  there): history speaks to the player's *trajectory* and what to do next
  session; the book tip speaks to *this session's* measured playing. Split
  homes also mean the two grounded voices never stack two appended lines
  onto one list.
- **LLM generator:** `pedagogy::tip_prompt_block` renders the tip into
  `build_recap_user_prompt` as GROUNDED INPUT (mirroring
  `insights::history_prompt_block`): topic + guidance + the attribution the
  model MUST keep visible, with invention of further book claims, exercise
  or page numbers, quotes, or other citations forbidden. No tip → no block
  (byte-identical prompt).
- **Thin recaps gain nothing, structurally:** both generators bail into
  `thin_session_recap` before any weaving (the #445-6b choke points), so a
  quick touch never gains a book line even when the command resolved a tip.

### 2.2 The coaching box (two voices, one box)
- Store: `coachingTip: MethodBookTip | null` (the S2 DTO shape: `topic`,
  `guidance`, `source_line`) beside `coachingSuggestion`.
  `refreshCoachingSuggestion` now fetches `method_book_tip` alongside
  `practice_suggestions` (same refresh points: session start + explore
  begin; one `Promise.allSettled`, each voice applied independently), under
  the same `_coachingFetchSeq` token — one boundary bump invalidates both
  in-flight voices.
- **Display policy: history outranks the tip.** When both exist the box
  shows the history suggestion — it is about *this player's* measured
  trajectory ("your Eb rows are sitting at 54%…"), while the book tip
  generalizes from one session's fingerprint to canonical technique
  guidance; the specific, personal claim wins the one slot. The tip fills
  the box otherwise. AT MOST ONE thing is ever shown.
- Rule 0, identical semantics, per voice: an empty/failed fetch never
  clears what a voice already holds; a newer result replaces in place;
  **dismissal quiets BOTH voices** for the session (one calm surface, one
  dismissal); session end / `openMatchedScore` reset both + the quiet.
- `CoachingBox` renders the tip with a **visible attribution line** —
  small, muted (`text-violet-400/70`), testid `coaching-box-attribution`,
  content `source_line`. Attribution is non-negotiable (#454). The history
  rendering is unchanged (its text embeds its own citations). No voice →
  no chrome, exactly as before.

## 3. Non-goals
New measurements or corpus entries (S4), refreshing the tip mid-session on
evidence changes (the tip the box shows is the S2 command's between-sessions
read; the recap's tip is the live one), a second box or stacked display,
any dim-on-contradiction state, teacher surfaces.

## 4. Acceptance criteria
1. The offline full recap appends exactly ONE method-book line — guidance
   verbatim with `source_line` in the copy — to `areas_to_improve`; no tip
   → no line (list unchanged).
2. A thin session's recap gains NO book line even when a tip exists.
3. The LLM user prompt carries the tip as GROUNDED INPUT (marked, guidance +
   attribution present, invention of further book claims/citations
   forbidden) when it exists, and no block when it doesn't.
4. The REAL end-session path over a live measured deficit (flat trumpet
   sustains) weaves exactly the one attributed Schlossberg line into the
   recap; the same deficit on a thin session weaves none.
5. The box surfaces the tip (guidance + visible attribution line with the
   muted class) when history has nothing to say, fetched via the
   `method_book_tip` command by name.
6. History outranks the tip: with both in the store the box shows the
   history suggestion, no attribution line, still exactly one box.
7. Rule 0 for the tip voice: an empty tip fetch never clears a shown tip;
   dismissal quiets BOTH voices against later fetches; session end resets
   both; the seq token invalidates an in-flight tip across a boundary.
8. Store wiring: the same refresh points fetch both commands; empty chrome
   stays empty (no suggestion AND no tip → the component renders nothing).
9. No #453 regression: every existing coaching-box / history-recap test
   stays green unmodified in meaning (fixture-only edits allowed).

## 5. Test map
| AC | Test |
|---|---|
| 1 | `coaching::tests::offline_recap_appends_attributed_method_book_line` |
| 2 | `coaching::tests::thin_recap_gains_no_method_book_line` |
| 3 | `coaching::tests::recap_prompt_carries_method_book_tip_as_grounded_input` |
| 4 | `commands::tests::end_session_weaves_attributed_method_book_line`, `commands::tests::thin_end_session_weaves_no_method_book_line` |
| 5 | CoachingBox.test.tsx `the method-book tip fills the box when history is silent — attribution visible` |
| 6 | CoachingBox.test.tsx `a history suggestion outranks the book tip` |
| 7 | practiceStore.test.ts `the tip voice holds through empty fetches and dismissal quiets both`, `a tip resolving after a session boundary writes nothing` |
| 8 | practiceStore.test.ts `session start fetches the method-book tip alongside history (#454 S3)`; CoachingBox.test.tsx `renders nothing when history has nothing to say` (extended: no tip either) |
| 9 | the untouched #453 suites (`coaching::tests::offline_recap_appends_at_most_one_history_suggestion`, CoachingBox.test.tsx, store #453 block) |

## 6. Architecture / notes
- The command layer stays the owner of catalog facts (family) and store
  reads; the brain stays pure over `RecapInput`. The one new pub seam is
  `coaching::build_fingerprint` — already the documented single assembly
  point for the fingerprint.
- `generate_recap_with_context` gains one parameter (8 total → a scoped
  `#[allow(clippy::too_many_arguments)]`, matching `build_recap`'s existing
  allow). The wrapper `generate_recap*` methods pass `None`.
- Offline-first: no new network; the tip is embedded corpus + local
  measurement end to end.
- References: issue #454, `docs/specs/454-s2-selection.md`,
  `docs/specs/453-s2-s3-recap-and-box.md`, `docs/specs/445-thin-recap.md`.
