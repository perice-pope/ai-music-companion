/**
 * Playwright test fixture for the GUI E2E suite.
 *
 * Every test gets a `page` that has, BEFORE any app code ran:
 *  - the recording mock Tauri IPC + the in-page outbound-network guard
 *    (`installTauriMockAndNetGuard`), and
 *  - a route-level abort of every request whose host is not the local
 *    preview origin. This is the second, independent offline guard: even
 *    if some code path slipped past the in-page `fetch`/XHR wrappers, the
 *    browser itself can't reach the network. Loopback (the Vite preview
 *    serving the app bundle) is allowed; everything else is aborted and
 *    recorded.
 *
 * `assertNoNetwork(page)` fails the test if either guard saw an outbound
 * attempt — that's the machine-checkable "works with no internet" gate.
 */
import { test as base, expect, type Page } from "@playwright/test";
import {
  installTauriMockAndNetGuard,
  type RecordedNetwork,
} from "../support/tauri-mock";

/** Hosts the app legitimately talks to: only the local preview server. */
const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** Requests Playwright aborted because their host was non-local. */
type AbortedRequest = { url: string; method: string };

export const test = base.extend<{ abortedRequests: AbortedRequest[] }>({
  abortedRequests: async ({ page }, use) => {
    const aborted: AbortedRequest[] = [];

    // Install the mock IPC + in-page net guard before app scripts execute.
    await page.addInitScript(installTauriMockAndNetGuard);

    // Route-level guard: abort anything that isn't the local preview origin
    // or an inert data:/blob: URL. data: is used by the in-page XHR guard's
    // redirect target, so it must be allowed through to resolve inertly.
    await page.route("**/*", (route) => {
      const url = new URL(route.request().url());
      if (url.protocol === "data:" || url.protocol === "blob:") {
        return route.continue();
      }
      if (LOCAL_HOSTS.has(url.hostname)) {
        return route.continue();
      }
      aborted.push({
        url: route.request().url(),
        method: route.request().method(),
      });
      return route.abort("blockedbyclient");
    });

    await use(aborted);
  },
});

export { expect };

/**
 * Read the in-page outbound-network log the mock installed. Empty ⇒ the
 * app made (well, attempted) no network calls.
 */
export async function getNetCalls(page: Page): Promise<RecordedNetwork[]> {
  return page.evaluate(() => window.__netCalls ?? []);
}

/** Read the ordered IPC call log the mock recorded. */
export async function getIpcCalls(page: Page): Promise<{ cmd: string }[]> {
  return page.evaluate(() =>
    (window.__ipcCalls ?? []).map((c) => ({ cmd: c.cmd })),
  );
}

/**
 * Assert no outbound network happened, via BOTH independent guards:
 * the in-page `fetch`/XHR/WS wrappers and Playwright's route-level abort.
 * Either being non-empty fails the test.
 */
export async function assertNoNetwork(
  page: Page,
  aborted: AbortedRequest[],
): Promise<void> {
  const inPage = await getNetCalls(page);
  expect(
    inPage,
    `in-page net guard recorded outbound attempt(s): ${JSON.stringify(inPage)}`,
  ).toEqual([]);
  expect(
    aborted,
    `route-level guard aborted non-local request(s): ${JSON.stringify(aborted)}`,
  ).toEqual([]);
}
