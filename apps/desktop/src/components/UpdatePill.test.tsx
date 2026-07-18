import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  act,
} from "@testing-library/react";

/**
 * #58 — the update pill: opt-in checks, one persistent element across
 * phases (#417 rule 0), per-version dismissal, calm failure.
 * The mocked plugin is the wire contract.
 */

const mockCheck = vi.fn();
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: () => mockCheck(),
}));

import UpdatePill from "./UpdatePill";
import { useUpdateStore } from "../stores/updateStore";
import { useConnectionsStore } from "../stores/connectionsStore";

function updateHandle(version: string) {
  return {
    version,
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
  };
}

beforeEach(() => {
  mockCheck.mockReset();
  localStorage.clear();
  useUpdateStore.setState({
    phase: "idle",
    availableVersion: null,
    notice: null,
    dismissedVersion: null,
  });
});

describe("UpdatePill (#58)", () => {
  it("renders nothing while idle — no empty chrome", () => {
    render(<UpdatePill />);
    expect(screen.queryByTestId("update-pill")).toBeNull();
  });

  it("a found update shows the pill with the version", async () => {
    mockCheck.mockResolvedValue(updateHandle("9.9.9"));
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());
    expect(screen.getByTestId("update-pill")).toBeInTheDocument();
    expect(screen.getByTestId("update-pill-install").textContent).toBe(
      "Update to v9.9.9",
    );
  });

  it("install transitions the SAME element through phases — rule 0", async () => {
    let resolveInstall: () => void = () => {};
    const handle = {
      version: "9.9.9",
      downloadAndInstall: vi.fn(
        () => new Promise<void>((res) => (resolveInstall = res)),
      ),
    };
    mockCheck.mockResolvedValue(handle);
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());

    const before = screen.getByTestId("update-pill");
    fireEvent.click(screen.getByTestId("update-pill-install"));
    await waitFor(() =>
      expect(screen.getByTestId("update-pill").dataset.phase).toBe(
        "downloading",
      ),
    );
    // Dimmed while working, never unmounted.
    expect(screen.getByTestId("update-pill")).toBe(before);
    expect(screen.getByTestId("update-pill").className).toContain("opacity-60");

    await act(async () => {
      resolveInstall();
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(screen.getByTestId("update-pill").dataset.phase).toBe("ready"),
    );
    expect(screen.getByTestId("update-pill")).toBe(before);
    expect(handle.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("update-pill-label").textContent).toContain(
      "Quit and reopen",
    );
  });

  it("dismiss hides that version; a NEWER version re-surfaces", async () => {
    mockCheck.mockResolvedValue(updateHandle("9.9.9"));
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());
    fireEvent.click(screen.getByTestId("update-pill-dismiss"));
    expect(screen.queryByTestId("update-pill")).toBeNull();

    // Same version again: stays hidden.
    mockCheck.mockResolvedValue(updateHandle("9.9.9"));
    await act(() => useUpdateStore.getState().checkForUpdate());
    expect(screen.queryByTestId("update-pill")).toBeNull();

    // A newer build: the pill earns its place back.
    mockCheck.mockResolvedValue(updateHandle("9.9.10"));
    await act(() => useUpdateStore.getState().checkForUpdate());
    expect(screen.getByTestId("update-pill-install").textContent).toBe(
      "Update to v9.9.10",
    );
  });

  it("a failed install shows a calm dimmed notice, no crash", async () => {
    const handle = {
      version: "9.9.9",
      downloadAndInstall: vi.fn().mockRejectedValue("network dropped"),
    };
    mockCheck.mockResolvedValue(handle);
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());
    fireEvent.click(screen.getByTestId("update-pill-install"));
    await waitFor(() =>
      expect(screen.getByTestId("update-pill").dataset.phase).toBe("error"),
    );
    expect(screen.getByTestId("update-pill").className).toContain("opacity-60");
    expect(screen.getByTestId("update-pill-label").textContent).toContain(
      "didn't finish",
    );
  });

  it("a failed CHECK stays silent — offline is normal", async () => {
    mockCheck.mockRejectedValue(new Error("offline"));
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());
    expect(screen.queryByTestId("update-pill")).toBeNull();
  });

  it("the auto-check flag is OFF by default and persists when set", () => {
    // AC1's foundation: the default must be false — the shipped promise
    // is no background update requests unless the user opts in.
    expect(useConnectionsStore.getState().autoUpdateCheckEnabled).toBe(false);
    useConnectionsStore.getState().setAutoUpdateCheckEnabled(true);
    expect(
      localStorage.getItem("ai-music-companion:auto-update-check-enabled"),
    ).toBe("true");
  });
});
