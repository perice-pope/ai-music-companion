---
description: Orchestrate a multi-slice epic across a fleet of parallel agents — plan, fan out, review, integrate, keep main green
argument-hint: <epic-issue-number>
---

You are the **tech lead + integration manager** for epic **#$ARGUMENTS**. You don't write the
feature code yourself — you decompose the work, fan out builder agents in parallel, gate every PR
with independent review, and serialize merges so `main` is always green. Follow `CLAUDE.md` →
"Engineering workflow & Definition of Done"; every slice still meets the full DoD. Quality is
NEVER traded for parallelism.

## The three hard constraints (respect them or the fleet collides)
1. **Dependencies.** A slice can't start until the slices it depends on are merged. Foundations
   (e.g. a shared engine/interface) are built **first, alone**, then dependents fan out.
2. **File overlap → merge conflicts.** Two slices that touch the same files must NOT run in the
   same wave — schedule them in different waves. Parallel only across **disjoint** file footprints.
3. **Compute.** Cap concurrent **heavy** (Rust/desktop-build) builders at ~3; frontend-only or
   docs slices can run wider. Don't exceed the cap even if more slices are "ready."

## Phase 0 — Plan (STOP for human approval before any code)
- `gh issue view $ARGUMENTS` — read the epic fully.
- Write/extend a spec at `docs/specs/$ARGUMENTS-<slug>.md` from `docs/specs/_TEMPLATE.md`. The
  **slice breakdown** must, per slice, state: a one-line goal, its **file/module footprint**, and
  its **depends-on** slice(s). Define shared **interfaces/contracts first** (the seams others build
  behind) as their own early slice.
- From that, produce the **wave plan**: order slices into waves where every slice in a wave has all
  deps merged and a footprint disjoint from the others in that wave.
- Present the spec + wave plan (as a table: slice → wave → footprint → depends-on) and **stop for
  approval** (plan mode). Do not build until approved.

## Phase 1 — Execute, wave by wave
For each wave (respecting the compute cap):
1. **Fan out builders in parallel.** For each slice, create an isolated worktree off the latest
   `main` (`git worktree add -b <branch> <path> origin/main`) and spawn a **builder agent** there.
   Each builder runs the per-slice `/feature` loop: build the slice → write behavior tests from the
   acceptance criteria → make `just ci` + desktop-manifest gates pass → push branch → open a PR
   linking #$ARGUMENTS. Builders must never push a red branch.
2. **Review every PR.** For each, run the `code-reviewer` and `test-auditor` agents adversarially
   against the spec. The builder addresses must-fix findings and re-greens the gates.
3. **Integrate — serialized, never parallel merges.** Merge the wave's PRs **one at a time**, in
   dependency order, only when CI is green (no `--admin`). **After each merge**, rebase the remaining
   open branches in the wave onto the new `main` and confirm they still build/pass — this surfaces
   conflicts immediately while they're small.
   - If a rebase conflict is trivial, resolve it. If it's non-trivial or risks correctness, **pause
     that slice, flag it to the human, and re-sequence** — do not guess a merge.
4. **Re-plan.** Merging a foundation unblocks its dependents → recompute the next wave's ready set.

## Phase 2 — Converge
- Repeat waves until every slice is merged. Keep `main` green the whole way.
- Verify the **epic's** acceptance criteria end-to-end (not just per-slice). Produce a single
  **manual-verify checklist** for the human and offer to build + launch the app to run it.
- Summarize: merged PRs, anything deferred (with reason), and close or update #$ARGUMENTS.

## Checkpoints with the human (don't run fully dark)
- After Phase 0: the plan must be approved.
- At each **wave boundary**: a short status + the manual-verify items for what just landed.
- Immediately on: a merge conflict you won't auto-resolve, a reviewer hard-block you can't satisfy,
  a gate you can't green, or any ambiguity in the spec. Surface it; don't paper over it.

## Guardrails
- Parallelism is for **disjoint** work only; overlapping slices serialize across waves.
- Merges are always serialized and `main` stays green; rebase open branches after each merge.
- Every slice passes the full DoD (gates + both reviewers + manual-verify). Parallelism must not
  lower the bar.
- Clean up worktrees as their PRs merge.
- For pure deterministic build/review fan-out within a wave you MAY use a Workflow; conflict
  resolution, merge ordering, and re-planning stay with you (the orchestrator).
