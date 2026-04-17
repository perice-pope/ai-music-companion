import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";

// Capture mock references so tests can override behavior
const mockInvoke = vi.fn().mockResolvedValue("pong");
const mockListen = vi.fn().mockResolvedValue(() => {});

// Mock Tauri invoke API
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Mock Tauri event API
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue("pong");
    mockListen.mockResolvedValue(() => {});
  });

  it("renders app title and phase indicator", () => {
    render(<App />);
    // getByRole/getByText throw if the element is missing — no extra assertion needed
    screen.getByRole("heading", { name: "AI Music Companion" });
    screen.getByText(/Phase 0/);
  });

  it("shows backend response after successful invoke", async () => {
    render(<App />);
    const response = await screen.findByTestId("backend-response");
    expect(response.textContent).toBe("Backend says: pong");
  });

  it("handles backend invoke failure gracefully", async () => {
    mockInvoke.mockRejectedValue(new Error("connection refused"));
    render(<App />);

    // App still renders its core UI despite the failed invoke
    screen.getByRole("heading", { name: "AI Music Companion" });

    // Wait a tick so the rejected promise settles
    await vi.waitFor(() => {
      // The backend-response element should never appear
      expect(screen.queryByTestId("backend-response")).toBeNull();
    });
  });

  it("sets up audio event listener on mount", async () => {
    render(<App />);

    // Wait for the async listen call inside useEffect
    await vi.waitFor(() => {
      expect(mockListen).toHaveBeenCalledTimes(1);
    });
    expect(mockListen).toHaveBeenCalledWith("audio-event", expect.any(Function));
  });
});
