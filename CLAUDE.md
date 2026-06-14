# CLAUDE.md — AI Music Companion

This file tells Claude Code how to work in this repository.

## Project overview

AI Music Companion is a desktop practice tool for musicians (brass, voice, strings, woodwinds, piano). It uses Tauri 2.0 + Rust backend + React frontend to deliver real-time audio analysis and feedback with <20ms latency.

Architecture: three layers — **Ears** (audio capture & analysis), **Brain** (score following, scoring, coaching), **Face** (UI).

## Repository layout

```
crates/ears/       — Rust: mic/MIDI capture, pitch detection, onset detection
crates/brain/      — Rust: score follower, scoring engine, practice planner
apps/desktop/      — Tauri 2.0 shell + React/TypeScript frontend
profiles/          — JSON instrument profiles (trumpet.json, voice.json, etc.)
```

## Commands

```bash
just ci            # Run full CI pipeline locally (fmt, clippy, test, audit, frontend lint+test+build)
just test          # cargo test --workspace && pnpm test
just bench         # Run latency benchmarks
just fmt           # cargo fmt && pnpm format
just lint          # cargo clippy --deny warnings && pnpm lint
just audit         # cargo audit && pnpm audit
```

## Conventions

### Commits
- Use **Conventional Commits**: `feat:`, `fix:`, `test:`, `docs:`, `refactor:`, `perf:`, `ci:`, `chore:`
- Keep commits atomic — one logical change per commit

### Rust
- Edition 2021, stable toolchain only
- `cargo clippy --deny warnings` must pass — no exceptions
- **NEVER allocate in the audio thread** (crates/ears real-time path). Use pre-allocated buffers, lock-free ring buffers, no `Box::new`, no `Vec::push`, no `String::from` in the hot path.
- Use `ringbuf` for SPSC communication between audio thread and processing thread
- Error handling: `thiserror` for library errors, `anyhow` for application-level
- Tests go in `crates/*/tests/` (integration) or inline `#[cfg(test)]` modules (unit)

### TypeScript / React
- Strict TypeScript — no `any`
- State management via Zustand
- Styling via Tailwind CSS
- Tests via Vitest + React Testing Library
- Package manager: **pnpm** (not npm, not yarn)

### Instrument profiles
- Adding a new instrument = adding a JSON file in `profiles/`. No code changes required.
- Profile schema: frequency range, vibrato tolerance, attack expectation, tuning corrections

## Latency budget

Total mic-to-screen must be **<25 ms**. Budget:
- Audio capture: ~5 ms (cpal buffer)
- Pitch detection: ~6 ms (Aubio yinfft hop)
- Score alignment: ~3 ms
- IPC + render: ~5 ms
- Headroom: ~6 ms

The CI benchmark suite measures the analysis path (samples → `AudioEvent`)
and **fails the build** if the mean exceeds 25 ms. The gate is enforced by
`.github/workflows/latency-bench.yml`, which runs `cargo bench --bench latency`
(see `crates/ears/benches/latency.rs`) on every PR touching `crates/ears/**`.

## Offline-first & network transparency

- **Offline by default — the internet is NEVER required for core value.** The full loop (capture → local analysis → recap) must work with zero network; LLM narration degrades to the on-device fallback (`coaching.rs`), never blocks, never fabricates.
- Every networked feature is **opt-in, off by default, and discloses what leaves the device** (to whom). Enumerate each in `docs/architecture/offline-first-and-network-transparency.md` and surface it in `ConnectionsPrivacy.tsx`. A new outbound call represented in neither is a bug.
- No telemetry-by-default, no silent network calls, no required account.

## What NOT to do

- Do not add dependencies without justification in the PR description
- Do not use `unsafe` in Rust without a `// SAFETY:` comment explaining why
- Do not put business logic in the frontend — it belongs in the Rust core
- Do not use Electron patterns (IPC should be thin JSON, not heavy serialization)
- Do not skip tests — every acceptance criterion from the story must have a corresponding test

## Engineering workflow & Definition of Done

Quality is enforced, not hoped for. Drive non-trivial work through `/feature <issue#>`,
which runs this loop. Hooks auto-format on edit and block finishing on a red format gate.

### The loop (spec → slice → build → verify → review → ship)
1. **Spec first.** Before code, write/confirm a spec from `docs/specs/_TEMPLATE.md` with
   **testable acceptance criteria**. No spec → no code.
2. **Slice small.** Decompose into vertical slices that each ship and review on their own
   (aim < ~400 changed lines per PR). One slice = one PR.
3. **Build the slice.**
4. **Test from the acceptance criteria.** Each AC maps to ≥1 test that asserts **behavior**
   and can fail for a real reason (see the test bar).
5. **Gates green locally.** Run `just ci` (fmt, clippy `-D warnings`, tests, audit,
   frontend lint/test/build) **and** the desktop-manifest checks until green. Never
   declare done on red.
6. **Independent review.** The adversarial `code-reviewer` and `test-auditor` agents review
   the diff against the spec. Address must-fix findings.
7. **Manual-verify.** Produce a short checklist of what a human should click/observe in the
   running app, and confirm behavior — not just green tests.
8. **Ship.** Open a PR linking the issue/slice with the DoD checklist.

### Definition of Done (a slice is NOT done until all are true)
- [ ] Every acceptance criterion has a behavior test that can fail for a real reason.
- [ ] `just ci` + desktop-manifest fmt/clippy/test pass locally.
- [ ] `code-reviewer` + `test-auditor` findings addressed (or deferred with a written reason).
- [ ] Manual-verify checklist run against the app.
- [ ] Conventional-commit message; PR links the issue and states scope + what's deferred.
- [ ] No new outbound network call without opt-in gating + disclosure (offline-first).

### Test quality bar (where "tests that pass but prove nothing" die)
- A test asserts **observable behavior or a contract** tied to an acceptance criterion —
  never just "a value exists" or "it didn't panic."
- Every test must be able to **fail for a real reason**: if you can't name the bug it would
  catch, it isn't a test.
- Cover **edges and failure modes**, not only the happy path.
- Assert **outputs/effects**, not implementation details, so honest refactors don't break tests.
- New behavior with no test you've seen fail first is a smell.

### Decomposition for the big features (#212–217, #208)
These are multi-slice epics — never one mega-PR. Spec the epic, then ship a sequence of small
PRs (e.g. accompaniment = audio-output engine → local synth → live tempo lock → opt-in cloud
bed). Each slice is independently testable and useful.
