import { test, expect, assertNoNetwork } from "../fixtures/app";

/**
 * Offline-by-default, proven at the UI level — the real Connections & Privacy
 * panel.
 *
 * The panel landed on `main` via PR #167 (`ConnectionsPrivacy.tsx` +
 * `connectionsStore.ts`). It is the Face-layer expression of the offline-first
 * principle (`docs/architecture/offline-first-and-network-transparency.md`):
 * offline by default, every networked feature opt-in and off until the user
 * turns it on, with plain-language disclosure of what leaves the device.
 *
 * This suite navigates to that real panel from the selector and asserts:
 *   1. boot/selector exposes no networked surface and makes no network;
 *   2. every networked toggle in the panel defaults OFF;
 *   3. the disclosure copy renders ("works offline" reassurance + a
 *      what's-sent / leaves-your-device statement);
 *   4. reaching and reading the whole panel triggers ZERO outbound HTTP.
 *
 * The toggles are native `<input type="checkbox" role="switch">`, so their
 * checked state lives in the accessibility tree (Playwright's `toBeChecked`),
 * not a literal `aria-checked` attribute — native checkboxes don't emit one.
 */

const PANEL = "connections-privacy-panel";

test.describe("Connections & Privacy (offline-by-default)", () => {
  test("no networked surface is reachable from boot, and nothing goes out", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");
    await expect(page.getByTestId("instrument-selector")).toBeVisible();

    // The boot/selector surface contains no auth/sync controls — the only
    // networked surface (AccountPanel sign-in) is on the History page and is
    // strictly opt-in. Assert it is NOT present on the default screen.
    await expect(page.getByLabel("Email")).toHaveCount(0);
    await expect(page.getByLabel("Password")).toHaveCount(0);

    // Hard offline gate: booting + rendering the selector made no network.
    await assertNoNetwork(page, abortedRequests);
  });

  test("Connections & Privacy panel: networked toggles default OFF + disclosure copy", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");
    await expect(page.getByTestId("practice-shell-selector")).toBeVisible();

    // Navigate to the real panel via the selector's entry point.
    await page.getByTestId("open-connections-privacy").click();

    const panel = page.getByTestId(PANEL);
    await expect(panel).toBeVisible();

    // Every networked toggle in the panel must default to OFF. The panel
    // exposes each as a native checkbox with role=switch, so its checked
    // state is read from the accessibility tree.
    const toggles = panel.getByRole("switch");
    const count = await toggles.count();
    expect(
      count,
      "panel should expose at least one networked toggle",
    ).toBeGreaterThan(0);
    for (let i = 0; i < count; i += 1) {
      await expect(
        toggles.nth(i),
        `networked toggle #${i} should default OFF`,
      ).not.toBeChecked();
    }

    // Disclosure copy, the heart of the privacy promise. The user can see
    // both that the app works offline AND what leaves the device when a
    // networked feature IS turned on — without trusting us blindly.
    await expect(
      panel.getByTestId("offline-reassurance"),
      "standing 'works offline' reassurance must always render",
    ).toContainText(/works offline/i);
    await expect(
      panel,
      "panel must disclose what is sent / leaves the device",
    ).toContainText(/leaves your device|it sends|sent to|what'?s sent/i);

    // Reaching and reading the entire panel triggered no outbound HTTP — the
    // disclosure screen itself honours the offline guarantee it describes.
    await assertNoNetwork(page, abortedRequests);
  });
});
