---
name: Story
about: A spec for the AI Music Companion build loop
title: "Story: "
labels: story
assignees: ''
---

## Story: <one-line title>

### Context
<!-- Why this exists — link to the architecture doc section -->

### Goal
<!-- What "done" looks like in one paragraph -->

### Acceptance Criteria (tests must cover all of these)
- [ ] Given X, when Y, then Z
- [ ] Performance: function X completes in <N ms on reference hardware
- [ ] ...

### Non-Goals
<!-- What NOT to build. Critical — prevents scope creep. -->

### Files likely to change
- `crates/...` (new/modified)
- `apps/desktop/src/...` (new/modified)

### Dependencies / Blocking stories
Depends on: #

### Definition of Done
- All acceptance criteria have passing tests
- `cargo clippy --deny warnings` clean
- Latency bench unchanged or improved
- PR description explains the change in plain English
