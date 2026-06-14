# Development quality system

This directory turns "prompt and pray" into an enforced engineering loop. See
`CLAUDE.md` → **Engineering workflow & Definition of Done** for the full contract.

## What's here
- **`commands/feature.md`** — the `/feature <issue#>` command. Drives **one slice** through
  spec → build → tests-from-acceptance-criteria → local gates → adversarial review → manual-verify
  → PR. The default way to build anything non-trivial.
- **`commands/epic.md`** — the `/epic <issue#>` orchestrator. The **dev-team** version: plans a
  multi-slice epic into a dependency/footprint graph, fans out builder agents in parallel **waves**
  (disjoint files only), reviews each PR, and **serializes merges** so `main` stays green. Use it
  for the big features (#208, #212–217); it calls the per-slice `/feature` loop under the hood.
- **`agents/code-reviewer.md`** — adversarial correctness reviewer (vs the spec's acceptance criteria).
- **`agents/test-auditor.md`** — audits tests for real rigor (catches tests that pass but prove nothing).
- **`hooks/format.sh`** — `PostToolUse` auto-formatter: rustfmt / prettier the file just edited.
- **`hooks/gate.sh`** — `Stop` gate: blocks finishing on unformatted Rust (the cheap, CI-accurate check).
- **`settings.json`** — wires the hooks.

## How to use it
1. Spec + plan: `/feature 212` → it writes `docs/specs/212-*.md`, proposes the first slice, and
   **stops for your approval** before coding.
2. It builds one small slice, tests it from the acceptance criteria, runs all gates, and has the
   `code-reviewer` + `test-auditor` review it adversarially.
3. It gives you a **manual-verify checklist** to run in the app, then opens the PR.

## Why
The point is separation of concerns + hard gates: a **spec** so tests mean something,
**independent review** so weak tests get caught, and **hooks** so a red build can't ship.
Heavy gates (clippy, tests, build) run in `/feature` and CI; the Stop hook only does the
fast format check so it never bogs down the loop.
