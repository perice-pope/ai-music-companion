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

const mockRelaunch = vi.fn();
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: () => mockRelaunch(),
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
  mockRelaunch.mockReset();
  mockRelaunch.mockResolvedValue(undefined);
  localStorage.clear();
  useUpdateStore.setState({
    phase: "idle",
    availableVersion: null,
    notice: null,
    releaseNotes: null,
    restartFailed: false,
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
    // E1: the pill click opens the window; Update now starts the install.
    fireEvent.click(screen.getByTestId("update-pill-install"));
    expect(screen.getByTestId("update-pill")).toBe(before); // grew in place
    fireEvent.click(screen.getByTestId("update-window-install"));
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
    // E1: ready now offers a real restart (the quit-and-reopen line is
    // the relaunch-failure fallback, tested below).
    expect(screen.getByTestId("update-pill-restart")).toBeInTheDocument();
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
    fireEvent.click(screen.getByTestId("update-window-install"));
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

  it("a late check resolution never stomps an in-flight download — TOCTOU", async () => {
    // Review MF2, proven at HEAD round 1: a heartbeat check in flight when
    // the user clicks install used to flip the phase back to "available",
    // re-enabling the button mid-download.
    let resolveInstall: () => void = () => {};
    const offered = {
      version: "9.9.9",
      downloadAndInstall: vi.fn(
        () => new Promise<void>((res) => (resolveInstall = res)),
      ),
    };
    mockCheck.mockResolvedValueOnce(offered);
    await act(() => useUpdateStore.getState().checkForUpdate());
    expect(useUpdateStore.getState().phase).toBe("available");

    // A heartbeat check goes airborne (deferred), THEN the user installs.
    let resolveLateCheck: (u: unknown) => void = () => {};
    mockCheck.mockImplementationOnce(
      () => new Promise((res) => (resolveLateCheck = res)),
    );
    const lateCheck = useUpdateStore.getState().checkForUpdate();
    const installing = useUpdateStore.getState().installUpdate();
    expect(useUpdateStore.getState().phase).toBe("downloading");

    // The stale check lands mid-download — it must not stomp the phase.
    await act(async () => {
      resolveLateCheck({
        version: "9.9.8",
        downloadAndInstall: vi.fn(),
      });
      await lateCheck;
    });
    expect(useUpdateStore.getState().phase).toBe("downloading");

    await act(async () => {
      resolveInstall();
      await installing;
    });
    expect(useUpdateStore.getState().phase).toBe("ready");
    expect(offered.downloadAndInstall).toHaveBeenCalledTimes(1);
  });

  it("double-clicking install downloads exactly once", async () => {
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
    fireEvent.click(screen.getByTestId("update-pill-install"));
    fireEvent.click(screen.getByTestId("update-window-install"));
    // The button is gone in the downloading phase, but pin the store too.
    await act(() => useUpdateStore.getState().installUpdate());
    expect(handle.downloadAndInstall).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolveInstall();
    });
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

  // ── #58 E1: the window ────────────────────────────────────────────────

  it("the window shows the release notes committed with the version", async () => {
    mockCheck.mockResolvedValue({
      version: "9.9.9",
      body: "• The Pocket learns to listen\n• Bug fixes",
      downloadAndInstall: vi.fn(),
    });
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());
    fireEvent.click(screen.getByTestId("update-pill-install"));
    expect(screen.getByTestId("update-window-title").textContent).toBe(
      "v9.9.9 — what's new",
    );
    expect(screen.getByTestId("update-window-notes").textContent).toContain(
      "The Pocket learns to listen",
    );
    // "Not now" collapses WITHOUT dismissing — the pill remains.
    fireEvent.click(screen.getByTestId("update-window-later"));
    expect(screen.queryByTestId("update-window")).toBeNull();
    expect(screen.getByTestId("update-pill-install")).toBeInTheDocument();
  });

  it("missing notes get the honest placeholder, never an empty pane", async () => {
    mockCheck.mockResolvedValue({
      version: "9.9.9",
      body: null,
      downloadAndInstall: vi.fn(),
    });
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());
    fireEvent.click(screen.getByTestId("update-pill-install"));
    expect(screen.getByTestId("update-window-notes").textContent).toBe(
      "No notes for this release.",
    );
  });

  it("Restart now relaunches; a relaunch failure falls back calmly", async () => {
    mockCheck.mockResolvedValue(updateHandle("9.9.9"));
    render(<UpdatePill />);
    await act(() => useUpdateStore.getState().checkForUpdate());
    await act(() => useUpdateStore.getState().installUpdate());
    fireEvent.click(screen.getByTestId("update-pill-restart"));
    await waitFor(() => expect(mockRelaunch).toHaveBeenCalledTimes(1));

    // Failure path: the calm quit-and-reopen line returns.
    mockRelaunch.mockRejectedValueOnce(new Error("no"));
    await act(() => useUpdateStore.getState().restartNow());
    await waitFor(() =>
      expect(screen.getByTestId("update-pill-label").textContent).toContain(
        "Quit and reopen",
      ),
    );
  });
});
