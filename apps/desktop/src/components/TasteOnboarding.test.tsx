import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import TasteOnboarding from "./TasteOnboarding";
import { useTasteStore } from "../stores/tasteStore";
import type { TasteProfile } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

/** Pull the profile out of the last `set_taste_profile` invoke call. */
function savedProfile(): TasteProfile {
  const call = mockInvoke.mock.calls.find((c) => c[0] === "set_taste_profile");
  if (!call) throw new Error("set_taste_profile was not invoked");
  return (call[1] as { profile: TasteProfile }).profile;
}

function advanceToEndAndFinish() {
  // genres → artists → goals → experience → Done
  fireEvent.click(screen.getByRole("button", { name: "Next" })); // -> artists
  fireEvent.click(screen.getByRole("button", { name: "Next" })); // -> goals
  fireEvent.click(screen.getByRole("button", { name: "Next" })); // -> experience
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
}

describe("TasteOnboarding", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue(undefined);
    // Reset the zustand store between tests.
    useTasteStore.setState({
      profile: {
        genres: [],
        artists: [],
        goals: [],
        experience: "beginner",
        is_under_13: false,
      },
      status: "idle",
      error: null,
      hasProfile: false,
    });
  });

  it("walks through every step and renders calm, non-judgemental copy", () => {
    render(<TasteOnboarding />);
    expect(screen.getByText("Let’s tune the coaching to you")).toBeTruthy();
    expect(screen.getByText("What do you love listening to?")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText("Any artists you’re into?")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText("What brings you here?")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText("Where are you on the journey?")).toBeTruthy();
  });

  it("captures selections and writes the profile locally via the command", async () => {
    const onComplete = vi.fn();
    render(<TasteOnboarding onComplete={onComplete} />);

    // Genres: tap two chips.
    fireEvent.click(screen.getByRole("button", { name: "Jazz" }));
    fireEvent.click(screen.getByRole("button", { name: "Hip-hop" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));

    // Artists: type + Add.
    fireEvent.change(screen.getByLabelText("Add an artist"), {
      target: { value: "Kendrick Lamar" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));

    // Goals: type + Add.
    fireEvent.change(screen.getByLabelText("Add a goal"), {
      target: { value: "audition prep" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));

    // Experience: choose a level, then Done.
    fireEvent.click(
      screen.getByRole("button", { name: /Getting comfortable/ }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Done" }));

    await waitFor(() => expect(onComplete).toHaveBeenCalled());

    const profile = savedProfile();
    expect(profile.genres).toEqual(["Jazz", "Hip-hop"]);
    expect(profile.artists).toEqual(["Kendrick Lamar"]);
    expect(profile.goals).toEqual(["audition prep"]);
    expect(profile.experience).toBe("intermediate");
    expect(profile.is_under_13).toBe(false);
  });

  it("lets an empty profile be saved (everything is optional)", async () => {
    const onComplete = vi.fn();
    render(<TasteOnboarding onComplete={onComplete} />);

    advanceToEndAndFinish();

    await waitFor(() => expect(onComplete).toHaveBeenCalled());
    const profile = savedProfile();
    expect(profile.genres).toEqual([]);
    expect(profile.experience).toBe("beginner");
  });

  it("removing a chip drops it from the saved profile", async () => {
    const onComplete = vi.fn();
    render(<TasteOnboarding onComplete={onComplete} />);

    fireEvent.click(screen.getByRole("button", { name: "Jazz" }));
    fireEvent.click(screen.getByRole("button", { name: "Jazz" })); // toggle off
    fireEvent.click(screen.getByRole("button", { name: "Pop" }));

    advanceToEndAndFinish();

    await waitFor(() => expect(onComplete).toHaveBeenCalled());
    expect(savedProfile().genres).toEqual(["Pop"]);
  });

  it("Skip for now completes without writing a profile", () => {
    const onComplete = vi.fn();
    render(<TasteOnboarding onComplete={onComplete} />);

    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));

    expect(onComplete).toHaveBeenCalled();
    expect(
      mockInvoke.mock.calls.some((c) => c[0] === "set_taste_profile"),
    ).toBe(false);
  });
});
