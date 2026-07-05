# Autonomy policy — how the nightly agent works, merges, and stays safe

This is the contract for the automated engineering agent (`.github/workflows/daily-agent.yml`).
The agent MUST read this file every run. Humans: change behavior by editing this file, not the
workflow prompt.

## Required reading (the context pack — every run, before any code)

Read these in order; they are the same context the founder's interactive sessions run with:

1. `CLAUDE.md` — house rules, gates, test bar, Definition of Done. Non-negotiable.
2. `docs/architecture/rv-methodology.md` — the product north star. The unit of practice is the
   CELL rowed through 12 keys; key detection is display honesty only. Design cell-first.
3. `docs/architecture/offline-first-and-network-transparency.md` — every networked feature is
   opt-in + disclosed. A new outbound call not in that table is a bug, not a feature.
4. `docs/design/decisions-log.md` — settled calls. Silence = "not decided, hands off."
5. `docs/specs/_TEMPLATE.md` — specs precede code; acceptance criteria must be testable.
6. This file.

Style: match the codebase, not your defaults. Comments state constraints the code can't show —
never narrate the next line or address a reviewer. Sleek, slim, simple is the founder's UI bar.

## Priorities (highest first)

1. **Red main.** If CI on `main` is failing, fixing it is the ONLY permitted work.
2. **Tester feedback** — open issues titled `[VA Test]…` or labeled `feedback`/`auto-fixable`.
   Reproduce from the report, fix the top finding, reference the issue.
3. **Hotspots** — `hotspot`-labeled issues (weekly CTO audit).
4. **Epic tail** — the open feature issues of the active epic, smallest first.
5. **Backlog stories** — the spec's next unbuilt slice (the original daily-agent behavior).

## The quality loop (no exceptions)

Spec (or confirm the issue IS the spec) → implement the smallest meaningful slice → tests per
acceptance criterion that can fail for a real reason → all local gates green (fmt, clippy
`-D warnings` workspace AND tauri manifest `--locked`, cargo test, pnpm lint/test/build/tsc) →
**adversarial self-review**: spawn an independent review agent on the full diff with the explicit
brief "find must-fix bugs and test gaps; try to refute the tests" — fix every must-fix it finds
before opening the PR, and say in the PR body what the review found.

## The merge envelope (evaluated by the workflow, not by you)

A deterministic workflow step — not the model — decides whether a PR auto-merges. In-envelope
PRs are marked ready and auto-merge once required CI passes; everything else stays a **draft**
for the founder. The envelope (all must hold):

- Diff ≤ 400 changed lines, ≤ 12 files.
- NO changes to: `Cargo.toml`/`package.json` dependencies, `pnpm-lock.yaml`, `Cargo.lock` beyond
  version-bump-free churn, `supabase/migrations/`, `.github/workflows/`, `CLAUDE.md`,
  `docs/autonomy/`, `va-testing-kit/*.sh`, release/version files.
- Branch CI fully green (enforced by branch protection regardless).

Widening this envelope is a founder-only edit to this file.

## Hard rules

- Never disable, skip, or weaken a failing check to get green.
- Never touch secrets, tokens, billing, or external services.
- Never delete user data or migrations.
- One slice per run. Nothing tractable → say so and exit; don't invent work.
- Every run ends with a summary the founder can read on a phone.
