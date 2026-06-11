import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright config for the GUI E2E suite (Option B: UI E2E against the
 * Vite web build with mocked IPC; native Tauri shell covered separately by
 * crates/brain/tests/e2e_offline.rs).
 *
 * `webServer` builds the production web bundle and serves it with
 * `vite preview`. Driving the *built* bundle (not the dev server) matters:
 *  - it's what ships, and
 *  - it dead-code-eliminates the app's own `devShim`, so the only IPC in
 *    play is our recording mock injected via `addInitScript`.
 *
 * Only Chromium is configured. The native webview (WebKit/WebView2) is the
 * Tauri shell's concern, out of scope for the web-build suite.
 */
const PORT = 4173;

export default defineConfig({
  testDir: "./tests",
  // Exclude the browserless Vitest unit file from Playwright's runner.
  testMatch: /.*\.spec\.ts$/,
  // Keep all generated artifacts under e2e/ (gitignored) regardless of the
  // cwd the runner is launched from (apps/desktop in CI).
  outputDir: `${__dirname}/test-results`,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI
    ? [
        ["list"],
        [
          "html",
          { open: "never", outputFolder: `${__dirname}/playwright-report` },
        ],
      ]
    : "list",
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "on-first-retry",
    // Belt-and-suspenders: deny camera/mic so nothing prompts headlessly.
    permissions: [],
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // Build the bundle then preview it. `--strictPort` so a stale server on
    // the port fails loudly instead of silently serving the wrong build.
    command: `pnpm --dir ../apps/desktop build && pnpm --dir ../apps/desktop exec vite preview --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
});
