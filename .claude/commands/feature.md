---
description: Drive an issue through the spec → slice → build → verify → review → ship loop
argument-hint: <issue-number> [slice note]
---

You are implementing GitHub issue **#$ARGUMENTS** under this repo's engineering workflow
(see `CLAUDE.md` → "Engineering workflow & Definition of Done"). Follow the loop exactly.
Do **not** shortcut it. If anything is ambiguous, stop and ask rather than guess.

## 0. Load context
- `gh issue view $ARGUMENTS` — read the issue fully.
- Check `docs/specs/` for an existing spec for this issue.

## 1. Spec first (STOP for approval if new)
- If no spec exists, write one at `docs/specs/$ARGUMENTS-<slug>.md` from `docs/specs/_TEMPLATE.md`
  — real, **testable acceptance criteria**, edge cases, a test plan mapping each AC → a test,
  and (for an epic) a **slice breakdown** of small shippable PRs.
- Present the spec and the proposed **first slice**, then **stop and get approval** before writing
  code. (Use plan mode.) No spec → no code.

## 2. Build the smallest slice
- Implement only the approved slice (aim < ~400 changed lines). Match the codebase's idioms.
- Respect the hard rules: no allocation in the audio thread; `unsafe` needs `// SAFETY:`;
  business logic in Rust core, not the frontend; offline-first (any new outbound call is opt-in,
  off by default, disclosed in `ConnectionsPrivacy` + the network allowlist).

## 3. Test from the acceptance criteria
- Write a behavior test for **each** acceptance criterion and edge case in the spec.
- Honor the test bar: assert observable behavior/contract, cover failure modes, and make sure
  each test can **fail for a real reason** (state, per test, what bug it catches).

## 4. Gates green locally (never declare done on red)
Run and make pass — fix and re-run until clean:
- `just ci` (fmt, clippy `-D warnings`, workspace tests, audit, frontend build)
- Desktop manifest: `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check`,
  `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib`
- If frontend touched: `pnpm -C apps/desktop lint && pnpm -C apps/desktop test && pnpm -C apps/desktop build && pnpm -C apps/desktop e2e:typecheck && pnpm -C apps/desktop e2e:unit`

## 5. Independent adversarial review (do NOT grade your own work)
Spawn **both** in parallel and address every must-fix finding (re-run gates after fixes):
- the `code-reviewer` agent — correctness vs the spec's acceptance criteria, edge cases, contract.
- the `test-auditor` agent — do the tests assert behavior (not existence)? Does every AC have a
  test that can fail for a real reason? Flag tautological/weak tests.

## 6. Manual-verify checklist (for the human)
Produce a short, concrete checklist of what to click/observe in the **running app** to confirm the
behavior end-to-end (not just green tests). Offer to build + launch the app for the user to run it.

## 7. Ship
- Conventional-commit on a `feat/<slug>` (or `fix/…`) branch; push; open a PR that links #$ARGUMENTS,
  states the slice scope and what's deferred, and includes the **Definition of Done** checklist with
  each box honestly checked.
- Report: PR URL, the exact gates you ran + results, reviewer findings + how addressed, and the
  manual-verify checklist. If a gate or review can't be satisfied, **stop and say so** — never ship red.
