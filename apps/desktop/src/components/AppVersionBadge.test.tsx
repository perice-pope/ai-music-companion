import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import AppVersionBadge from "./AppVersionBadge";
import { getVersion } from "@tauri-apps/api/app";

// The badge exists so a tester can quote the build they're running (#384).
// It must show the bundle's own version — and must NEVER become an error
// surface when the shell API is missing (browser preview / failed IPC).

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(),
}));
const getVersionMock = vi.mocked(getVersion);

describe("AppVersionBadge", () => {
  beforeEach(() => {
    getVersionMock.mockReset();
  });

  it("shows the bundle version once getVersion resolves", async () => {
    getVersionMock.mockResolvedValue("2.29.0");
    render(<AppVersionBadge />);
    expect(await screen.findByTestId("app-version")).toHaveTextContent(
      "v2.29.0",
    );
  });

  it("renders nothing while the version is still loading", () => {
    getVersionMock.mockReturnValue(new Promise(() => {})); // never settles
    const { container } = render(<AppVersionBadge />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing when the shell API is absent (browser preview)", async () => {
    getVersionMock.mockRejectedValue(new Error("no __TAURI_INTERNALS__"));
    const { container } = render(<AppVersionBadge />);
    // Let the rejection settle; the badge must stay silent, not crash or
    // print an error.
    await waitFor(() => expect(getVersionMock).toHaveBeenCalled());
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });

  it("passes the caller's positioning classes through", async () => {
    getVersionMock.mockResolvedValue("2.29.0");
    render(<AppVersionBadge className="mt-8 text-center" />);
    const badge = await screen.findByTestId("app-version");
    expect(badge.className).toContain("mt-8");
    expect(badge.className).toContain("text-center");
  });
});
