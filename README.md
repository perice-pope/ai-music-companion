# AI Music Companion

An intelligent desktop practice companion for musicians — brass, voice, strings, woodwinds, and piano.

Built with **Tauri 2.0** (Rust backend + React frontend) for sub-20ms latency real-time audio feedback.

## Architecture

| Layer | What it does | Location |
|-------|-------------|----------|
| **Ears** | Mic/MIDI capture, pitch detection, onset detection | `crates/ears/` |
| **Brain** | Score following, per-note scoring, practice planning | `crates/brain/` |
| **Face** | Sheet music display, feedback UI, pitch meter | `apps/desktop/` |

## Quick start

```bash
# In a GitHub Codespace (recommended) or local with Rust + Node installed:
just ci          # Run full CI pipeline
just dev         # Start the dev server
just test        # Run all tests
```

## Development

- **Task runner:** [just](https://github.com/casey/just) — see `justfile` for all commands
- **Commits:** [Conventional Commits](https://www.conventionalcommits.org/)
- **Package manager (frontend):** pnpm
- **Cloud dev:** Open in GitHub Codespaces for zero-setup development

## Instrument profiles

Each instrument is defined by a JSON profile in `profiles/`. Adding a new instrument = adding a JSON file. No code changes required.

## Project docs

- [Architecture](ai-music-companion-architecture.md)
- [SDLC Automation Loop](sdlc-automation-loop.md)
- [Claude Code Rules](CLAUDE.md)
