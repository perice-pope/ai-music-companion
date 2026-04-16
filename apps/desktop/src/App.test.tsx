import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";

// Mock Tauri invoke API
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue("pong"),
}));

// Mock Tauri event API
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders without crashing", () => {
    render(<App />);
    expect(screen.getByText("AI Music Companion")).toBeDefined();
  });

  it("displays the phase indicator", () => {
    render(<App />);
    expect(screen.getByText(/Phase 0/)).toBeDefined();
  });

  it("displays backend response after invoke resolves", async () => {
    render(<App />);
    const response = await screen.findByTestId("backend-response");
    expect(response.textContent).toBe("Backend says: pong");
  });
});
