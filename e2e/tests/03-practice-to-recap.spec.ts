import { test, expect, assertNoNetwork, getIpcCalls } from "../fixtures/app";

/**
 * The core practice → recap flow, end to end, with NO network.
 *
 * Headless there is no real microphone, so live pitch/phrase events never
 * fire — exactly as in the seeded backend. The recap is produced by the
 * mocked `end_practice_session` IPC response (a real-shaped
 * `brain::session::SessionRecap`). This exercises the real React screens,
 * the Zustand state machine (selector → session → recap), and the recap
 * rendering, all over the IPC boundary with zero HTTP.
 *
 * Live coaching (PR #171): the CoachingTipPanel IS rendered on main, but
 * without live phrase events no tips are pushed, so the panel shows its
 * empty state. We assert the panel exists if present and skip gracefully
 * otherwise — never block on coaching wiring.
 */
test.describe("practice → recap", () => {
  test("select instrument, practice, end, and see the recap — offline", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");

    // 1. Pick an instrument.
    await page.getByTestId("instrument-card-trumpet").click();
    await expect(page.getByTestId("selected-indicator")).toBeVisible();

    // 2. Start practice → session screen.
    await page.getByTestId("start-practice-button").click();
    await expect(page.getByTestId("practice-session")).toBeVisible();
    await expect(page.getByTestId("session-timer")).toBeVisible();

    // Coaching panel: present-on-main, empty without live events. Don't block.
    const coaching = page.getByTestId("coaching-tip-panel-empty");
    if ((await coaching.count()) > 0) {
      await expect(coaching.first()).toBeVisible();
    }

    // 3. End the session → recap screen, driven by seeded recap IPC.
    await page.getByTestId("end-session-button").click();

    const recap = page.getByTestId("session-recap");
    await expect(recap).toBeVisible();
    await expect(recap).toHaveAttribute("data-variant", "summary");

    // Seeded recap content renders.
    await expect(page.getByTestId("recap-assessment")).toContainText(
      /focused session/i,
    );

    // Product invariant: strengths render BEFORE areas in the DOM.
    const strengths = page.getByTestId("recap-strengths");
    const areas = page.getByTestId("recap-areas");
    await expect(strengths).toBeVisible();
    await expect(areas).toBeVisible();
    const order = await recap.evaluate((el) => {
      const s = el.querySelector('[data-testid="recap-strengths"]');
      const a = el.querySelector('[data-testid="recap-areas"]');
      if (!s || !a) return "missing";
      // Node.DOCUMENT_POSITION_FOLLOWING (4) ⇒ a comes after s.
      return s.compareDocumentPosition(a) & Node.DOCUMENT_POSITION_FOLLOWING
        ? "strengths-first"
        : "areas-first";
    });
    expect(order).toBe("strengths-first");

    // The fingerprint dimensions the seeded recap carries are shown.
    await expect(page.getByTestId("recap-intonation")).toBeVisible();
    await expect(page.getByTestId("recap-groove")).toBeVisible();

    // 4. The whole flow rode entirely over IPC — assert the calls happened…
    const ipc = await getIpcCalls(page);
    const cmds = ipc.map((c) => c.cmd);
    expect(cmds).toContain("start_practice_session");
    expect(cmds).toContain("end_practice_session");

    // …and that NOTHING went out over the network during any of it.
    await assertNoNetwork(page, abortedRequests);
  });

  test("recap actions return to the selector — still offline", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");
    await page.getByTestId("instrument-card-trumpet").click();
    await page.getByTestId("start-practice-button").click();
    await page.getByTestId("end-session-button").click();
    await expect(page.getByTestId("session-recap")).toBeVisible();

    await page.getByTestId("recap-done").click();
    await expect(page.getByTestId("practice-shell-selector")).toBeVisible();

    await assertNoNetwork(page, abortedRequests);
  });
});
