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

## What NOT to do

- Do not add dependencies without justification in the PR description
- Do not use `unsafe` in Rust without a `// SAFETY:` comment explaining why
- Do not put business logic in the frontend — it belongs in the Rust core
- Do not use Electron patterns (IPC should be thin JSON, not heavy serialization)
- Do not skip tests — every acceptance criterion from the story must have a corresponding test
