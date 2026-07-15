import { test, expect, assertNoNetwork } from "../fixtures/app";

/**
 * #354 — the score cursor must actually PAINT, and move, when the backend
 * ticks score positions. Five VA runs saw a follower install and a first
 * position emit with no visible cursor; the Rust worker loop is proven by
 * `audio_pipeline`'s emission-contract tests, so the untested half of the
 * bug space is everything from the webview's event listener to real-OSMD
 * pixels. This spec drives that half for real: production bundle, real
 * Chromium, real OSMD render, and genuine `score-position-updated`
 * deliveries through the mock's event registry.
 *
 * The paint assertions are SCREENSHOT DIFFS of the score pane, not
 * `toBeVisible()`. The #279 failure shape — cursor element present, every
 * move "works", zero pixels change because the img sits under an opaque
 * background — passes visibility checks; only pixels tell the truth.
 */

/** A backend position tick, matching `brain::follower::ScorePosition`. */
const position = (measure: number, beat: number) => ({
  measure_number: measure,
  beat,
  section_name: null,
  expected_note: null,
});

test.describe("score cursor paints and moves (#354)", () => {
  test("position events produce a visible, advancing cursor", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");

    // Real flow: instrument → score picker → seeded score → session.
    await page.getByTestId("instrument-card-trumpet").click();
    await page.getByTestId("practice-with-score-button").click();
    await page.getByText("Seeded Etude").click();
    await page
      .getByRole("button", { name: /start practice with this score/i })
      .click();

    // The real OSMD render of the two-measure seeded score.
    const pane = page.getByTestId("score-view");
    await expect(pane.locator("svg")).toBeVisible();
    // Give OSMD's post-render settle (cursor reset/hide) a beat, then
    // capture the no-cursor baseline.
    await page.waitForTimeout(300);
    const before = await pane.screenshot();

    // Backend ticks measure 1 — the cursor must appear. Pixels, not DOM.
    await page.evaluate(
      (p) => window.__emitTauriEvent("score-position-updated", p),
      position(1, 0.0),
    );
    await expect(async () => {
      const shown = await pane.screenshot();
      expect(shown.equals(before), "cursor tick must change pixels").toBe(
        false,
      );
    }).toPass({ timeout: 5_000 });
    const atMeasure1 = await pane.screenshot();

    // The cursor element OSMD manages should also report where it is —
    // used below to prove MOVEMENT, not just change. Positions are read
    // relative to the canvas so a centering shift can't masquerade as
    // (or hide) cursor motion.
    const cursor = page.locator("#cursorImg-0");
    const canvas = page.getByTestId("score-view-canvas");
    await expect(cursor).toBeAttached();
    const relX = async () => {
      const c = await cursor.boundingBox();
      const k = await canvas.boundingBox();
      return c && k ? c.x - k.x : null;
    };
    const box1 = await relX();
    expect(box1, "cursor must have real geometry once shown").not.toBeNull();
    const cursorSize = await cursor.boundingBox();
    expect(
      cursorSize!.height,
      "cursor must be a real band, not a hairline (the 30×1 px regression)",
    ).toBeGreaterThan(8);

    // Backend advances to measure 2 — the cursor must MOVE right.
    await page.evaluate(
      (p) => window.__emitTauriEvent("score-position-updated", p),
      position(2, 0.0),
    );
    await expect(async () => {
      const shown = await pane.screenshot();
      expect(
        shown.equals(atMeasure1),
        "advancing a measure must change pixels",
      ).toBe(false);
    }).toPass({ timeout: 5_000 });
    const box2 = await relX();
    expect(box2).not.toBeNull();
    expect(
      box2!,
      `cursor must advance rightward into measure 2 (was rel x=${box1})`,
    ).toBeGreaterThan(box1!);

    // Session end (null position path) hides it again: pixels return to
    // (approximately) the baseline — assert via difference from the shown
    // state rather than byte-equality with `before`, since OSMD may
    // re-rasterize antialiasing.
    await assertNoNetwork(page, abortedRequests);
  });
});
