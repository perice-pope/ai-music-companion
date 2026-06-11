import { defineConfig } from "vitest/config";

/**
 * Browserless unit checks for the E2E mock logic. Kept separate from the
 * Playwright runner so the load-bearing IPC-mock behaviour has runnable
 * coverage even where a real browser binary can't be fetched.
 */
export default defineConfig({
  test: {
    include: ["tests/**/*.unit.test.ts"],
    environment: "node",
  },
});
