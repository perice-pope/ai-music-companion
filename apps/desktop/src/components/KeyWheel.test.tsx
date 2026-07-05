import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import KeyWheel from "./KeyWheel";
import type { WheelViewDto } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

function wheel(overrides?: Partial<WheelViewDto>): WheelViewDto {
  return {
    cells: Array.from({ length: 12 }, (_, tonic) => ({
      tonic,
      state: "none" as const,
      attempts: 0,
      best_accuracy: 0,
      scales: [],
    })),
    intonation_trend: "unknown",
    tone_trend: "unknown",
    total_owned: 0,
    ...overrides,
  };
}

describe("KeyWheel (#256)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // First run: all 12 cells render in the resting state with the invite copy —
  // a defined empty state, no crash, no fake glow.
  it("renders a defined 12-cell empty state", async () => {
    mockInvoke.mockResolvedValueOnce(wheel());
    render(<KeyWheel />);
    await waitFor(() =>
      expect(screen.getByTestId("key-wheel")).toBeInTheDocument(),
    );
    for (let pc = 0; pc < 12; pc++) {
      expect(screen.getByTestId(`wheel-cell-${pc}`)).toHaveAttribute(
        "data-state",
        "none",
      );
    }
    expect(screen.getByText("play to light up")).toBeInTheDocument();
    expect(screen.queryByTestId("wheel-trends")).not.toBeInTheDocument();
  });

  // Mastery states drive the cells; owned count + trends surface.
  it("lights owned and learning cells and shows trends", async () => {
    const view = wheel({ total_owned: 1, intonation_trend: "improving" });
    view.cells[7] = {
      tonic: 7,
      state: "owned",
      attempts: 5,
      best_accuracy: 0.92,
      scales: ["dorian"],
    };
    view.cells[0] = {
      tonic: 0,
      state: "learning",
      attempts: 2,
      best_accuracy: 0.6,
      scales: ["major"],
    };
    mockInvoke.mockResolvedValueOnce(view);
    render(<KeyWheel />);
    await waitFor(() =>
      expect(screen.getByTestId("wheel-cell-7")).toHaveAttribute(
        "data-state",
        "owned",
      ),
    );
    expect(screen.getByTestId("wheel-cell-0")).toHaveAttribute(
      "data-state",
      "learning",
    );
    expect(screen.getByText(/1 of 12 owned/)).toBeInTheDocument();
    expect(screen.getByTestId("wheel-trends")).toHaveTextContent(
      "Intonation: improving",
    );
  });

  // Tap a key → its detail (drills, best %, scales); tap again → closes.
  it("tapping a cell toggles its detail", async () => {
    const view = wheel();
    view.cells[7] = {
      tonic: 7,
      state: "owned",
      attempts: 5,
      best_accuracy: 0.92,
      scales: ["dorian", "major"],
    };
    mockInvoke.mockResolvedValueOnce(view);
    render(<KeyWheel />);
    await waitFor(() =>
      expect(screen.getByTestId("key-wheel")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByTestId("wheel-cell-7"));
    const detail = screen.getByTestId("wheel-detail");
    expect(detail).toHaveTextContent("G — owned ✨");
    expect(detail).toHaveTextContent("5 drills · best 92%");
    expect(detail).toHaveTextContent("dorian, major");
    fireEvent.click(screen.getByTestId("wheel-cell-7"));
    expect(screen.queryByTestId("wheel-detail")).not.toBeInTheDocument();
  });

  // A failed fetch renders nothing — the selector stays calm.
  it("renders nothing when the snapshot fails", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("no backend"));
    const { container } = render(<KeyWheel />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(container.querySelector("[data-testid=key-wheel]")).toBeNull();
  });
});
