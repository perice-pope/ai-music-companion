# AI Music Companion

**Real-time practice coaching for musicians.**

> [**View the pitch deck →**](https://perice-pope.github.io/ai-music-companion-pitch/)
>
> A quick walk-through of the vision, market, and product direction. The deck lives in its own public repo: [perice-pope/ai-music-companion-pitch](https://github.com/perice-pope/ai-music-companion-pitch).

## Overview

AI Music Companion is a desktop practice tool for brass, voice, strings, woodwinds, and piano players. It listens to your playing, tracks pitch and timing against a score, and delivers phrase-level coaching feedback -- all with sub-25ms mic-to-screen latency. Built with Tauri 2.0 (Rust backend, React frontend) so the audio-critical path stays in native code while the UI remains fast to iterate on.

## Architecture

```
  +-----------+      +-----------+      +-----------+
  |   Ears    | ---> |   Brain   | ---> |   Face    |
  | (capture) |      | (analysis)|      |   (UI)    |
  +-----------+      +-----------+      +-----------+
```

- **Ears** -- Audio capture and pitch/onset detection. Real-time thread with zero allocations.
- **Brain** -- Score following, per-note scoring, phrase analysis, and AI coaching engine.
- **Face** -- React UI with live pitch display, sheet music view, and practice feedback.

## Tech Stack

| Layer | Technology |
|---|---|
| Backend | Rust, Tauri 2.0 |
| Audio | cpal, ringbuf, YIN pitch detection |
| Frontend | React, TypeScript, Tailwind CSS, Zustand |
| AI Coaching | Claude / GPT-4 API (phrase-level feedback) |
| Storage | SQLite (local), Supabase (cloud sync) |
| CI/CD | GitHub Actions, semantic-release |

## Project Status

| Phase | Description | Status |
|---|---|---|
| 0 | Spike -- architecture validation | Complete |
| 1 | Practice Companion MVP | In Progress |
| 2 | Smart Import + Tone Quality | Planned |
| 3 | Teacher Platform + Mobile | Planned |

## Repository Layout

```
crates/ears/       -- Audio capture, pitch detection, onset detection
crates/brain/      -- Score following, phrase analysis, coaching engine
apps/desktop/      -- Tauri 2.0 shell + React/TypeScript frontend
profiles/          -- JSON instrument profiles
docs/              -- Testing guides, architecture docs
```

## Development

### Setup

```bash
# Prerequisites: Rust stable, Node 20+, pnpm

pnpm install                       # Frontend dependencies
cargo build                        # Rust workspace
cargo test --workspace             # Run all tests
pnpm --filter desktop test         # Frontend tests
```

### Environment Variables

To enable LLM-powered coaching and session recaps, set your Claude API key:

```bash
export MUSIC_COMPANION_LLM_API_KEY="sk-ant-..."
```

(Get your API key at https://console.anthropic.com/)

When the API key is not set, the app runs in **offline mode**: sessions still work fully, but coaching tips are unavailable and recaps use fallback text. Set the env var before running the app to enable coaching.

### Common Commands

A [justfile](justfile) provides shortcuts for common workflows:

```bash
just ci       # Full CI pipeline (fmt, clippy, test, audit, frontend lint+test+build)
just test     # cargo test --workspace && pnpm test
just lint     # cargo clippy --deny warnings && pnpm lint
just fmt      # cargo fmt && pnpm format
just bench    # Latency benchmarks (fails if >25ms budget exceeded)
```

## Contributing

- Use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `test:`, etc.)
- `cargo clippy --deny warnings` must pass -- no exceptions
- **Never allocate in the audio thread** (`crates/ears/` real-time path). No `Box::new`, no `Vec::push`, no `String::from` in the hot path.
- Every acceptance criterion needs a corresponding test

See [CLAUDE.md](CLAUDE.md) for the full set of coding conventions and project rules.

## License

This project is not yet licensed. A license will be added before public release.
