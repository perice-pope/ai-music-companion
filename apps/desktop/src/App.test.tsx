import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";

const mockInvoke = vi.fn();
const mockListen = vi.fn().mockResolvedValue(() => {});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue("unused");
    mockListen.mockResolvedValue(() => {});
  });

  it("renders the practice shell with the app title on the selector screen", () => {
    render(<App />);
    // getByRole throws if missing — no extra assertion needed.
    screen.getByRole("heading", { name: "AI Music Companion" });
    screen.getByText(/Free Play/);
  });

  it("sets up the audio-event listener on mount", async () => {
    render(<App />);
    await vi.waitFor(() => {
      expect(mockListen).toHaveBeenCalledTimes(1);
    });
    expect(mockListen).toHaveBeenCalledWith("audio-event", expect.any(Function));
  });

  it("still renders the UI when the event subscription fails", async () => {
    // Design invariant: a broken audio event bus must not take down the
    // whole app — the selector is still reachable and the user can
    // still pick an instrument.
    mockListen.mockRejectedValueOnce(new Error("IPC not ready"));
    render(<App />);
    screen.getByRole("heading", { name: "AI Music Companion" });
    screen.getByText(/Free Play/);
  });
});
