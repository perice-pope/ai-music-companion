import { test, expect, assertNoNetwork } from "../fixtures/app";

/**
 * #184 — "Start Practice with This Score" must actually start a session.
 *
 * It was a silent no-op: the picker guarded on `practiceStore.instrumentName`,
 * which is null until a session is already running, so the click always
 * early-returned. This drives the real flow — pick an instrument → score
 * picker → pick the seeded score → the green button is enabled and issues
 * `start_practice_session` WITH the chosen score id (the proof it's no longer
 * dead). No unit test would have caught this; it needs the real screen wiring.
 */
test.describe("start practice with a score (#184)", () => {
  test("the green button starts a session with the chosen score", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");

    // An instrument must be picked to reach the score picker.
    await page.getByTestId("instrument-card-trumpet").click();
    await expect(page.getByTestId("selected-indicator")).toBeVisible();
    await page.getByTestId("practice-with-score-button").click();

    // Pick the seeded library score → it becomes the active score and the
    // "Start Practice with This Score" panel appears.
    await page.getByText("Seeded Etude").click();

    const startBtn = page.getByRole("button", {
      name: /start practice with this score/i,
    });
    await expect(startBtn).toBeEnabled();

    // Reaching here (picker + score load) rode entirely over IPC — assert the
    // offline guarantee before we start the session (which renders the score
    // and is out of scope for this regression).
    await assertNoNetwork(page, abortedRequests);

    await startBtn.click();

    // The click actually issues start_practice_session WITH the score id —
    // the regression proof. (Pre-fix, no such call ever fired.)
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window.__ipcCalls ?? []).some(
            (c) =>
              c.cmd === "start_practice_session" &&
              Boolean((c.args as { scoreId?: string } | undefined)?.scoreId),
          ),
        ),
      )
      .toBe(true);
  });
});
