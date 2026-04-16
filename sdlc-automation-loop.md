# SDLC Automation Loop — Building the AI Music Companion with Claude Code

**Companion doc to:** `ai-music-companion-architecture.md`
**Status:** Proposed v1.0
**Date:** April 15, 2026

---

## Layman's overview (read this first)

> We're going to set up a **factory line** where:
> 1. One bot writes a **spec** (a clear description of one small piece of the app).
> 2. Claude Code reads that spec, **writes the code and the tests**, runs everything, and only finishes if the tests pass.
> 3. Claude Code opens a **pull request** on GitHub with a clean commit message.
> 4. A second bot (an AI code reviewer) reads the pull request and leaves comments — just like a senior engineer would.
> 5. If the review is clean, we **merge into main, tag a release**, and the line moves to the next spec.
> 6. All of this runs in the **cloud** (GitHub Codespaces — free, no AWS account needed) so your laptop doesn't melt.
>
> Each section below has a 🎯 pointer so you can jump around.

---

## Section 1 — The cloud dev environment (no AWS, no laptop meltdown) 🎯 Infrastructure

### Recommendation: **GitHub Codespaces** (Docker-based, runs in the cloud, free tier)

| Option | Verdict |
|---|---|
| **GitHub Codespaces** ✅ | Runs a Docker devcontainer *in GitHub's cloud*. 120 core-hours/month free on personal accounts. Already integrates with the GitHub account you'll be using for PRs anyway. Zero infra to manage. **Pick this.** |
| Docker Desktop (local) | Still uses your laptop's CPU/RAM. Defeats the purpose. |
| Gitpod | Good alternative if you ever leave GitHub — but since we're on GitHub for PRs/CI, Codespaces is one less account. |
| AWS / EC2 | Overkill. Requires IAM, billing alarms, a VPC… you don't want this. |
| Railway / Fly.io | Great for hosting apps, not great for IDE-style dev containers. |

### How it works in plain English

A file named `.devcontainer/devcontainer.json` lives in the repo. It tells Codespaces "spin up a Docker image with Rust, Node, Python, and these tools." When you (or a bot) open the repo in Codespaces, GitHub boots a container **in their data center**, you SSH/VSCode into it, and all compiles/tests run there. Your laptop only runs a thin web or VSCode client.

### The Tauri-specific wrinkle (and the fix)

Tauri builds a desktop GUI, and cloud containers don't have a monitor. That's fine because **95% of what needs testing is headless**:

- **Rust core** (audio DSP, score follower, scoring engine) → normal `cargo test`, runs anywhere.
- **Frontend logic** → Vitest / React Testing Library, headless.
- **End-to-end UI** → `tauri-driver` + **WebDriver** running under `xvfb` (virtual display) in the container. Screenshots saved as CI artifacts.
- **Real mic/MIDI hardware tests** → run on your laptop **once per release candidate**, not every commit.

### Cost ceiling

Set a Codespaces spending limit of **$0** in GitHub settings. You'll get 120 free core-hours/month on a 2-core machine = 60 hours of active dev. If a bot runs too hot, GitHub just pauses it — no surprise bill.

---

## Section 2 — The SDLC loop 🎯 Process

```
   ┌────────────────────────────────────────────────────────────┐
   │  1. SPEC AUTHOR  (Claude, runs weekly or on-demand)        │
   │     Reads architecture doc + backlog                       │
   │     Produces one GitHub Issue: "Story #42 — cpal mic       │
   │     capture with ring buffer"                              │
   │     Uses a strict template (see Section 3)                 │
   └───────────────────────────┬────────────────────────────────┘
                               │
                               ▼
   ┌────────────────────────────────────────────────────────────┐
   │  2. BUILDER  (Claude Code, in a Codespace)                 │
   │     - git checkout -b story/42-cpal-mic-capture            │
   │     - Reads the spec's Acceptance Criteria                 │
   │     - Writes tests first, then code                        │
   │     - Runs `just ci` locally until green                   │
   │     - Commits with Conventional Commit message             │
   │     - git push, gh pr create                               │
   └───────────────────────────┬────────────────────────────────┘
                               │
                               ▼
   ┌────────────────────────────────────────────────────────────┐
   │  3. CI GATE  (GitHub Actions)                              │
   │     - cargo fmt, cargo clippy --deny warnings              │
   │     - cargo test, cargo audit                              │
   │     - pnpm lint, pnpm test, pnpm build                     │
   │     - Latency benchmark (fails if >25 ms mic→event)        │
   │     - Tauri E2E under xvfb                                 │
   └───────────────────────────┬────────────────────────────────┘
                               │  green
                               ▼
   ┌────────────────────────────────────────────────────────────┐
   │  4. AI CODE REVIEW  (Claude GitHub App + CodeRabbit)       │
   │     Posts PR comments: bugs, security, style, test gaps    │
   │     Builder auto-responds to fixable comments              │
   └───────────────────────────┬────────────────────────────────┘
                               │  approved
                               ▼
   ┌────────────────────────────────────────────────────────────┐
   │  5. HUMAN STAMP  (you, ~30 seconds)                        │
   │     Look at PR summary + AI review, click "Squash & merge" │
   │     (Or set branch protection to auto-merge on approval)   │
   └───────────────────────────┬────────────────────────────────┘
                               │
                               ▼
   ┌────────────────────────────────────────────────────────────┐
   │  6. RELEASE BOT  (GitHub Action on merge to main)          │
   │     - semantic-release: bumps version, writes CHANGELOG    │
   │     - Tags v0.x.y, creates GitHub Release                  │
   │     - Triggers next story from the backlog queue           │
   └───────────────────────────┬────────────────────────────────┘
                               │
                               └──────► back to step 1
```

---

## Section 3 — The Spec template (what a "story" looks like) 🎯 Inputs to Claude Code

Every issue the Spec Author creates follows this template. Claude Code reads it like a contract:

```markdown
## Story: <one-line title>

### Context
<Why this exists — links to architecture doc section>

### Goal
<What "done" looks like in one paragraph>

### Acceptance Criteria (tests must cover all of these)
- [ ] Given X, when Y, then Z
- [ ] Performance: function X completes in <5 ms on reference hardware
- [ ] ...

### Non-Goals
<What NOT to build. Critical — prevents scope creep.>

### Files likely to change
- `crates/ears/src/capture.rs` (new)
- `crates/ears/tests/capture_test.rs` (new)

### Dependencies / Blocking stories
Depends on: #38

### Definition of Done
- All acceptance criteria have passing tests
- `cargo clippy --deny warnings` clean
- Latency bench unchanged or improved
- PR description explains the change in plain English
```

The Spec Author agent is prompted with the architecture doc + the list of already-completed stories, and emits one of these per run.

---

## Section 4 — AI code review in GitHub 🎯 Quality gate

### Two bots, belt-and-suspenders:

1. **Claude for GitHub** (official Anthropic app). Install once on the repo. On every PR it posts a review summarizing intent, flagging bugs, and suggesting improvements. Can also be mentioned (`@claude please look at the ring buffer logic`) for deeper dives.
2. **CodeRabbit** (free tier for open-source, cheap for private). Complements Claude with style/security linting and inline suggestions. Different model, different blind spots — catches things the other misses.

### Guardrails so reviews actually matter:

- Branch protection on `main`: requires 1 AI review + CI green + 1 human approval (you).
- Builder bot is allowed to auto-apply "committable suggestions" from the reviewers, then re-runs CI.
- Reviewers are instructed (via repo `CLAUDE.md` and `.coderabbit.yaml`) to weight: correctness > security > test coverage > performance > style.

---

## Section 5 — Repo layout & key files 🎯 What to create

```
ai-music-companion/
├── .devcontainer/
│   └── devcontainer.json          ← Codespaces config (Rust+Node+Python)
├── .github/
│   ├── workflows/
│   │   ├── ci.yml                 ← lint, test, bench
│   │   ├── release.yml            ← semantic-release
│   │   └── spec-author.yml        ← scheduled bot that opens new issues
│   ├── ISSUE_TEMPLATE/story.md    ← the spec template from Section 3
│   └── pull_request_template.md
├── CLAUDE.md                      ← project rules for Claude Code
├── crates/                        ← Rust core (ears, brain, ...)
├── apps/desktop/                  ← Tauri shell + React frontend
├── profiles/                      ← JSON instrument profiles
├── justfile                       ← one-shot commands: `just ci`, `just bench`
└── README.md
```

The `CLAUDE.md` is the most important file — it tells every Claude Code run: which crates exist, which commands to use, which patterns to follow, and which things to *never* do (e.g., "never allocate in the audio thread").

---

## Section 6 — The full stable toolchain for the loop 🎯 Picks

| Job | Tool | Why it's the boring/safe choice |
|---|---|---|
| Cloud dev env | **GitHub Codespaces** | Managed Docker, free tier, same account as your repo |
| Container base | `mcr.microsoft.com/devcontainers/rust` + Node & Python features | Microsoft-maintained, rebuilt weekly |
| CI | **GitHub Actions** | Native, free for private repos up to 2000 min/mo |
| Commit convention | **Conventional Commits** | Machine-parseable, feeds semantic-release |
| Versioning & releases | **semantic-release** | Auto-bumps semver, writes CHANGELOG, creates tags |
| PR reviews | **Claude for GitHub** + **CodeRabbit** | Two perspectives, both free tiers |
| Task runner | **just** (justfile) | Simpler than Make, one command per verb |
| Headless UI tests | **tauri-driver** + **WebDriverIO** + **xvfb** | Official Tauri path |
| Audit | **cargo audit**, **pnpm audit** | Runs in CI, fails build on CVEs |
| Backlog | **GitHub Projects** (built-in) | No extra tool, kanban view of stories |

---

## Section 7 — Step-by-step "Day 0" checklist 🎯 What you actually click

1. Create empty GitHub repo `ai-music-companion` (private).
2. Click **Code → Open in Codespace** (creates the free container).
3. Drop in the `.devcontainer/devcontainer.json`, `CLAUDE.md`, `justfile`, and workflow YAMLs (I can generate these next).
4. Install **Claude for GitHub** and **CodeRabbit** apps on the repo (two clicks each).
5. Enable branch protection on `main`: require status checks + 1 approving review.
6. Set Codespaces spending limit to **$0** in Settings → Billing → Spending limits.
7. Add 3–5 starter stories (the Phase-0 spike from the architecture doc) as issues using the template.
8. Kick off the first Claude Code run against story #1. Watch the loop turn.

---

## Section 8 — What could go wrong & how we catch it 🎯 Risks

| Risk | Mitigation |
|---|---|
| Bot merges broken code | AI review + branch protection + required CI + your 30-second human stamp |
| Codespaces quota burns out mid-month | Spending limit = $0; loop pauses instead of billing |
| Flaky tests erode trust | CI retries once; persistent flakes auto-file a bug story |
| Spec quality drifts | Spec Author reads the architecture doc every run; you review the issue before Claude Code starts |
| Supply-chain attack via new dep | `cargo audit` + Dependabot + PR review flags new deps for human check |

---

## One-line summary

Set up a GitHub repo with **Codespaces for cloud dev, a spec-template-driven issue → Claude Code → CI → AI review → semantic-release loop**, protected by branch rules and backed by two free AI reviewers — giving you a hands-off build pipeline for the AI Music Companion with **no AWS account and no strain on your laptop**.
