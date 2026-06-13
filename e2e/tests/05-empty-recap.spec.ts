import { test, expect, assertNoNetwork } from "../fixtures/app";

/**
 * #185 / #186 — a session that formed no phrases must still render a calm,
 * readable recap, never the accusatory "you didn't play".
 *
 * Headless has no mic, so no live phrases ever form — the recap is whatever the
 * backend returns. The seeded backend returns the duration-aware empty-state
 * recap for a Voice session (phrase_count 0, real duration), so this exercises:
 *  - #185 Part B: the calm "couldn't pick out phrases" copy with a real
 *    duration, instead of "you didn't get to play"; and
 *  - #186: the recap rendering on its own dark surface (not light-on-white).
 */
test.describe("empty-state recap (#185/#186)", () => {
  test("renders the calm empty recap on a dark surface — offline", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");

    await page.getByTestId("instrument-card-voice").click();
    await page.getByTestId("start-practice-button").click();
    await expect(page.getByTestId("practice-session")).toBeVisible();
    await page.getByTestId("end-session-button").click();

    const recap = page.getByTestId("session-recap");
    await expect(recap).toBeVisible();
    await expect(recap).toHaveAttribute("data-variant", "empty");

    // #185: the calm mic-check copy, never the accusatory "didn't play".
    const assessment = page.getByTestId("recap-assessment");
    await expect(assessment).toContainText(/pick out distinct phrases/i);
    await expect(assessment).not.toContainText(/didn't get to play/i);

    // #186: the recap sits on a dark full-height surface (its wrapper),
    // not the browser's default white.
    const hasDarkBg = await recap.evaluate(
      (el) => el.parentElement?.classList.contains("bg-gray-900") ?? false,
    );
    expect(hasDarkBg).toBe(true);

    await assertNoNetwork(page, abortedRequests);
  });
});
