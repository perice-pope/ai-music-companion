import { describe, it, expect } from "vitest";
import { render, waitFor, cleanup, act } from "@testing-library/react";
import ScoreView, { type OsmdLike, type OsmdFactory } from "./ScoreView";
import type { ScorePosition } from "../types/brain";

/**
 * OSMD cursor performance regression guard.
 *
 * HONESTY / SCOPE: jsdom has no layout engine and no real frame clock, so the
 * wall-clock numbers here are NOT representative of real 60fps rendering on a
 * GPU. Real-frame-rate cursor-follow under load is a hardware check — see
 * `docs/qa-runbook.md` ("OSMD cursor-follow at 60fps"). What this test DOES
 * guard is the *algorithm* in ScoreView's `moveCursorToMeasure`: it must stay
 * roughly linear in the distance walked (never re-walk the whole score per
 * advance) and complete a long session of advances quickly. A quadratic
 * regression — e.g. resetting to measure 0 on every forward tick — would blow
 * the step budget below long before it ever showed up on real hardware.
 *
 * We can't unit-test the real OSMD cursor without a browser, so we exercise
 * the *real* component code path (`moveCursorToMeasure`, the forward-walk and
 * rewind-via-reset logic) against a faithful fake whose `next()`/`reset()`
 * mirror real OSMD semantics (forward-only, parks at the end). The number we
 * assert on is `next()` call count — a clock-independent proxy for the
 * algorithm's cost — plus a generous wall-clock ceiling as a coarse backstop.
 */

const LARGE_MEASURE_COUNT = 250; // 250 measures ≈ a long etude / movement.

/**
 * A faithful fake OSMD that mirrors the real cursor's semantics:
 *  - forward-only `next()` that parks at the final measure,
 *  - `reset()` jumps back to measure 0,
 *  - a 0-based `currentMeasureIndex` iterator.
 * It records every `next()` so the test can assert the *number of steps*
 * the component took — the load-bearing, clock-independent signal.
 */
function makeLargeFakeOsmd(measureCount: number) {
  let measure = 0;
  let nextCalls = 0;
  let resetCalls = 0;
  const osmd: OsmdLike = {
    async load() {},
    render() {},
    clear() {},
    cursor: {
      show() {},
      hide() {},
      reset() {
        resetCalls += 1;
        measure = 0;
      },
      next() {
        nextCalls += 1;
        if (measure < measureCount - 1) measure += 1;
      },
      get iterator() {
        return { currentMeasureIndex: measure };
      },
    },
  };
  return {
    factory: (() => osmd) as OsmdFactory,
    currentMeasure: () => measure,
    nextCalls: () => nextCalls,
    resetCalls: () => resetCalls,
    // Zero the counters — call after the initial load+render settles so the
    // assertions measure only the *seek-driven* work, not the one-time
    // `cursor.reset()` the load effect performs (effect 1 in ScoreView).
    zeroCounters: () => {
      nextCalls = 0;
      resetCalls = 0;
    },
  };
}

function pos(measure: number): ScorePosition {
  return { measure_number: measure, beat: 0 };
}

// Minimal valid-shaped MusicXML; the fake never actually parses it.
const XML = `<?xml version="1.0"?><score-partwise version="3.1"><part-list>
<score-part id="P1"><part-name>M</part-name></score-part></part-list>
<part id="P1"><measure number="1"><note><rest/><duration>4</duration></note></measure></part>
</score-partwise>`;

describe("ScoreView — OSMD cursor performance regression guard", () => {
  it("advances monotonically through a large score in linear total steps", async () => {
    const fake = makeLargeFakeOsmd(LARGE_MEASURE_COUNT);
    const { rerender } = render(
      <ScoreView
        musicXml={XML}
        cursorPosition={null}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(0));
    // Ignore the one-time load-effect reset; measure only the seek work.
    fake.zeroCounters();

    // Drive the cursor forward one measure at a time across the whole score
    // — the common live-following case (the follower nudges us forward).
    const advances = LARGE_MEASURE_COUNT - 1;
    const start = performance.now();
    for (let m = 2; m <= LARGE_MEASURE_COUNT; m += 1) {
      rerender(
        <ScoreView
          musicXml={XML}
          cursorPosition={pos(m)}
          osmdFactory={fake.factory}
        />,
      );
      // Flush the cursor effect for this tick before the next rerender.
      await act(async () => {});
    }
    const elapsedMs = performance.now() - start;

    // Landed on the last measure (0-based index = count - 1).
    await waitFor(() =>
      expect(fake.currentMeasure()).toBe(LARGE_MEASURE_COUNT - 1),
    );

    // CORE GUARD: a forward-only monotonic walk must take exactly one
    // `next()` per measure crossed — total ≈ advances, NOT advances².
    // We allow a small constant of slack but cap well under the quadratic
    // blow-up (which would be ~advances²/2 ≈ 31k steps for 250 measures).
    expect(fake.nextCalls()).toBeLessThanOrEqual(advances + 5);
    // Forward-only walk should never reset.
    expect(fake.resetCalls()).toBe(0);

    // Coarse wall-clock backstop. jsdom timing is not real-frame-rate, so
    // this is deliberately generous — it only catches gross regressions
    // (e.g. an accidental O(n²) seek), not true 60fps adherence.
    expect(elapsedMs).toBeLessThan(2000);
    cleanup();
  });

  it("handles repeated rewinds without unbounded step growth", async () => {
    // Jumping backward (repeats / re-alignment) forces reset()+forward-walk.
    // Each rewind to measure k costs ~k steps; this guards that a rewind
    // doesn't somehow walk the *entire* score or loop forever.
    const fake = makeLargeFakeOsmd(LARGE_MEASURE_COUNT);
    const { rerender } = render(
      <ScoreView
        musicXml={XML}
        cursorPosition={pos(200)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() => expect(fake.currentMeasure()).toBe(199));
    // Ignore the initial load reset + first forward walk; measure the rounds.
    fake.zeroCounters();

    const REWIND_TARGET = 50; // 0-based index 49
    const ROUNDS = 20;
    const start = performance.now();
    for (let r = 0; r < ROUNDS; r += 1) {
      // Rewind, then advance forward again, alternating.
      rerender(
        <ScoreView
          musicXml={XML}
          cursorPosition={pos(REWIND_TARGET)}
          osmdFactory={fake.factory}
        />,
      );
      await act(async () => {});
      rerender(
        <ScoreView
          musicXml={XML}
          cursorPosition={pos(200)}
          osmdFactory={fake.factory}
        />,
      );
      await act(async () => {});
    }
    const elapsedMs = performance.now() - start;

    // Each round: walk back via reset (~49 steps) + forward to 200 (~150).
    // Bound total at a comfortable linear multiple of the per-round cost so
    // a regression that re-walks far more is caught.
    const perRoundUpperBound = LARGE_MEASURE_COUNT + REWIND_TARGET + 10;
    expect(fake.nextCalls()).toBeLessThanOrEqual(perRoundUpperBound * ROUNDS);
    // One reset per rewind round (forward-only cursor) — not more.
    expect(fake.resetCalls()).toBe(ROUNDS);
    expect(elapsedMs).toBeLessThan(2000);
    cleanup();
  });

  it("never spins the guard cap when asked to seek past the end", async () => {
    const fake = makeLargeFakeOsmd(LARGE_MEASURE_COUNT);
    render(
      // Way past the end — the guard cap (MAX_STEPS=10_000) must not engage;
      // the walk must stop when the iterator parks at the final measure.
      <ScoreView
        musicXml={XML}
        cursorPosition={pos(99_999)}
        osmdFactory={fake.factory}
      />,
    );
    await waitFor(() =>
      expect(fake.currentMeasure()).toBe(LARGE_MEASURE_COUNT - 1),
    );
    // Stopped at the parked end after ~measureCount steps, nowhere near the cap.
    expect(fake.nextCalls()).toBeLessThan(LARGE_MEASURE_COUNT + 5);
    cleanup();
  });
});
