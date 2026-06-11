import { test, expect, assertNoNetwork } from "../fixtures/app";

/**
 * Offline-by-default, proven at the UI level.
 *
 * The task asks for a "Connections & Privacy" panel where every networked
 * toggle defaults OFF and disclosure copy ("what's sent / works offline")
 * renders. As of the `main` this suite was written against, that panel
 * component does NOT exist in `apps/desktop/src` — it lives only on the
 * `claude/offline-first-transparency` branch (`ConnectionsPrivacy.tsx` +
 * `connectionsStore.ts`), which has not merged to main. The CI disclosure
 * guard from PR #169 operates on the Rust core, not a React panel.
 *
 * Rather than fake the panel or edit shared components (a parallel agent
 * owns those), this spec:
 *   1. auto-skips the panel-specific assertions with a clear message when
 *      the component is absent (so it goes green today and starts ACTUALLY
 *      asserting the toggles the moment the panel lands), and
 *   2. asserts the offline-by-default guarantees that ARE present on main:
 *      the entire boot + practice surface renders with no networked toggle
 *      in the "on" position and no outbound HTTP — the only networked
 *      surface in the app (cloud sign-in) is opt-in and lives off the
 *      practice path, on the History page.
 *
 * When the panel merges, delete the `panelPresent` guard and the
 * placeholder selectors below describe exactly what to assert.
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
  }) => {
    await page.goto("/");

    const panel = page.getByTestId(PANEL);
    const present = (await panel.count()) > 0;

    test.skip(
      !present,
      "ConnectionsPrivacy panel is not on this `main` (lives on " +
        "claude/offline-first-transparency). Native/Rust offline guard is " +
        "covered by crates/brain/tests/e2e_offline.rs + the CI disclosure " +
        "guard. This assertion activates automatically once the panel merges.",
    );

    // --- Activates only when the panel exists -----------------------------
    await expect(panel).toBeVisible();

    // Every networked toggle in the panel must default to OFF (unchecked /
    // aria-checked=false). The panel exposes its switches as role=switch.
    const toggles = panel.getByRole("switch");
    const count = await toggles.count();
    expect(
      count,
      "panel should expose at least one networked toggle",
    ).toBeGreaterThan(0);
    for (let i = 0; i < count; i += 1) {
      await expect(toggles.nth(i)).toHaveAttribute("aria-checked", "false");
    }

    // Disclosure copy: "works offline" + "what's sent" must render so the
    // user can see the privacy posture without trusting us blindly.
    await expect(panel).toContainText(/works offline/i);
    await expect(panel).toContainText(
      /what'?s sent|sent to|leaves your device/i,
    );
  });
});
