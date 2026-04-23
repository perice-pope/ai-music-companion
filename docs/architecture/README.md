# Architecture

This folder holds the living architecture for Musa / AI Music Companion.

## Files

| File | What it is |
|------|------------|
| [`architecture-v2.md`](./architecture-v2.md) | **Current spec.** The "coach, don't judge" architecture — this is the one to read. |
| [`architecture-v1.md`](./architecture-v1.md) | Original pre-pivot architecture. Kept for diff / decision tracking. |
| [`research-notes.md`](./research-notes.md) | Early product + market research that fed into v1/v2. |
| [`platform-modules-addendum.md`](./platform-modules-addendum.md) | Addendum to v2 — modular platform thinking (teacher/student/group modes, tone-quality model, etc.). |
| [`eyes.md`](./eyes.md) | RFC (exploration): computer vision as a third sensor for technique analysis — landmarks live, VLM async. |
| [`mobile.md`](./mobile.md) | RFC (committed): iOS + Android via Tauri 2. iPad-first for schools, then phone. |
| [`sdlc-automation-loop.md`](./sdlc-automation-loop.md) | How we build: daily engineering agent, weekly CTO audit, testing standards, PR hygiene. |

## Related

- [`../design/`](../design/) — per-story design docs (e.g. `story-14-free-play-mode.md`)
- [`../design/decisions-log.md`](../design/decisions-log.md) — running record of non-obvious product + engineering calls
- [`../testing-standards.md`](../testing-standards.md) — what "a real test" means here
- [`../../CLAUDE.md`](../../CLAUDE.md) — project rules the agents (and humans) follow
