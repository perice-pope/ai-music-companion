# Roadmap — how to drive the remaining work

This is the run-top-to-bottom checklist for finishing the app. It's **a few focused
sessions, not one overnight run** — bounded by your testing time, paid-service
decisions, and compute. Day to day, your input is: paste a command, glance at the
plan, say "approved, proceed," and test the result.

See `CLAUDE.md` → "Engineering workflow & Definition of Done" and `.claude/README.md`
for how `/feature` (one slice) and `/epic` (a whole feature, fleet of agents) work.

## Start each session in the app repo
```bash
cd ~/Documents/musa-app && claude
```
(The `/feature` / `/epic` commands and the auto-format/gate hooks only activate when
Claude Code is rooted in this repo.)

## Run order (dependency-correct)
Each command plans first, **shows you the plan to approve**, then runs its fleet and
checks in at wave boundaries. Blocked items wait on their foundation automatically.

### Wave 1 — offline & independent (no keys, no decisions — safest to run first)
- [ ] `/epic 212` — **Audio Output & Sound Engine + accompaniment** (the foundation + flagship). The local-synth path needs no keys; the Suno-class bed is a later opt-in slice.
- [ ] `/epic 216` — "Your one thing today" (adaptive daily plan; fully offline)
- [ ] `/epic 217` — Progress you can feel (highlight reel; fully offline)
- [ ] `/feature 210` — Practice-mode per-phrase cadence (small)

### Wave 2 — depends on the audio engine (#212) landing
- [ ] `/epic 213` — "Hear it fixed" (corrected playback)
- [ ] `/epic 215` — Talk-to-your-coach (voice) — **needs API keys** (LLM + ElevenLabs-class TTS + STT)

### Wave 3 — the moat
- [ ] `/epic 208` — Real idiom corpus + encoder — **needs dataset/licensing decisions**
- [ ] `/epic 214` — Song identification → score auto-load (after #208)

## The 3 things only you can decide (no automation replaces these)
1. **API keys + cost** — Suno (accompaniment), ElevenLabs (voices), the LLM (coach) are
   paid cloud services. The local/offline baselines build without them; premium *sound*
   needs your keys and a cost call. They stay opt-in, off by default, disclosed (offline-first).
2. **Dataset licensing** for the moat (#208/#214) — sourcing real reference recordings is a
   business/legal decision, not a code one.
3. **Your ears** — whether the accompaniment *feels* good and the voice *sounds* real is the
   manual-verify step. Only you can judge it; that judgment is the point.

## To put in as little as possible
- Keep **auto-merge on** so green + reviewed slices land without you.
- At each plan checkpoint, if it looks right: **"approved, proceed."**
- Run **Wave 1** first — zero keys or decisions, moves the app forward hands-off.
- Save 208 / 214 / 215 for once keys + licensing are sorted.

## Definition of done for the whole app
Every issue above merged to `main`, each having passed the full DoD (behavior tests from
acceptance criteria, gates green, both reviewers, manual-verify), with `main` green throughout
and the paid-service features shipping their offline baseline + opt-in premium layer.
