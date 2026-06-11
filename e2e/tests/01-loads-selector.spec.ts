import { test, expect, assertNoNetwork, getIpcCalls } from "../fixtures/app";

/**
 * Smoke: the app boots to the instrument selector with zero network.
 *
 * This is the entry point of the offline-by-default promise — the very
 * first screen renders entirely from the (mocked) Rust core over IPC, with
 * no HTTP at all.
 */
test.describe("app boot", () => {
  test("loads to the instrument selector with no network", async ({
    page,
    abortedRequests,
  }) => {
    await page.goto("/");

    // The selector shell and the instrument selector both render.
    await expect(page.getByTestId("practice-shell-selector")).toBeVisible();
    await expect(page.getByTestId("instrument-selector")).toBeVisible();

    // The seeded catalog came across the IPC boundary and rendered.
    await expect(page.getByTestId("instrument-card-trumpet")).toBeVisible();
    await expect(page.getByTestId("instrument-card-voice")).toBeVisible();

    // Catalog populated via IPC, not HTTP.
    const ipc = await getIpcCalls(page);
    expect(ipc.map((c) => c.cmd)).toContain("list_instruments");

    await assertNoNetwork(page, abortedRequests);
  });
});
