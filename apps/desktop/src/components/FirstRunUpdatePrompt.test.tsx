import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";

/**
 * #465 — the once-only first-launch question about automatic update checks.
 * The contract under test: it asks exactly once, either answer is final and
 * persisted, "no thanks" changes nothing else, and it never fights the pill
 * for the bottom-left slot.
 */

import FirstRunUpdatePrompt from "./FirstRunUpdatePrompt";
import { useConnectionsStore } from "../stores/connectionsStore";
import { useUpdateStore } from "../stores/updateStore";

const AUTO_UPDATE_KEY = "ai-music-companion:auto-update-check-enabled";
const PROMPT_KEY = "ai-music-companion:update-prompt-answered";

beforeEach(() => {
  localStorage.clear();
  useConnectionsStore.setState({
    autoUpdateCheckEnabled: false,
    updatePromptAnswered: false,
  });
  useUpdateStore.setState({ phase: "idle" });
});

describe("FirstRunUpdatePrompt (#465)", () => {
  it("asks the question on a fresh install, with both answers offered", () => {
    render(<FirstRunUpdatePrompt />);
    // The founder's copy: what it asks, what it costs, where to change it.
    screen.getByText(/Check for updates automatically\?/);
    screen.getByText(/One request to GitHub for a version file/);
    screen.getByText(/Connections & Privacy/);
    screen.getByTestId("update-prompt-yes");
    screen.getByTestId("update-prompt-no");
  });

  it("yes enables the toggle, persists both flags, and retires the prompt", () => {
    render(<FirstRunUpdatePrompt />);
    fireEvent.click(screen.getByTestId("update-prompt-yes"));

    const state = useConnectionsStore.getState();
    expect(state.autoUpdateCheckEnabled).toBe(true);
    expect(state.updatePromptAnswered).toBe(true);
    // Persisted, so next launch neither re-asks nor forgets the opt-in.
    expect(localStorage.getItem(AUTO_UPDATE_KEY)).toBe("true");
    expect(localStorage.getItem(PROMPT_KEY)).toBe("true");
    expect(screen.queryByTestId("first-run-update-prompt")).toBeNull();
  });

  it("no thanks records only the answer — everything else stays as today", () => {
    render(<FirstRunUpdatePrompt />);
    fireEvent.click(screen.getByTestId("update-prompt-no"));

    const state = useConnectionsStore.getState();
    expect(state.autoUpdateCheckEnabled).toBe(false);
    expect(state.updatePromptAnswered).toBe(true);
    // "No" must not write the toggle's key at all — declining is not a
    // setting change, and the untouched key is the proof.
    expect(localStorage.getItem(AUTO_UPDATE_KEY)).toBeNull();
    expect(localStorage.getItem(PROMPT_KEY)).toBe("true");
    expect(screen.queryByTestId("first-run-update-prompt")).toBeNull();
  });

  it("never asks twice — an answered question stays gone for good", () => {
    useConnectionsStore.setState({ updatePromptAnswered: true });
    render(<FirstRunUpdatePrompt />);
    expect(screen.queryByTestId("first-run-update-prompt")).toBeNull();

    // Even a user who later opts in and back out again in Connections &
    // Privacy must not be re-asked.
    act(() => {
      useConnectionsStore.getState().setAutoUpdateCheckEnabled(true);
      useConnectionsStore.getState().setAutoUpdateCheckEnabled(false);
    });
    expect(screen.queryByTestId("first-run-update-prompt")).toBeNull();
  });

  it("an explicit Connections & Privacy choice answers the question too", () => {
    render(<FirstRunUpdatePrompt />);
    screen.getByTestId("first-run-update-prompt");

    // Turning the toggle OFF (already its value) is still an explicit,
    // informed choice — asking afterwards would nag.
    act(() => {
      useConnectionsStore.getState().setAutoUpdateCheckEnabled(false);
    });

    expect(useConnectionsStore.getState().updatePromptAnswered).toBe(true);
    expect(localStorage.getItem(PROMPT_KEY)).toBe("true");
    expect(screen.queryByTestId("first-run-update-prompt")).toBeNull();
  });

  it("stays away when auto-check is already on, even unanswered", () => {
    // e.g. a pre-#465 install that opted in via settings: the question is
    // moot and showing it would ask about a switch that's already flipped.
    useConnectionsStore.setState({ autoUpdateCheckEnabled: true });
    render(<FirstRunUpdatePrompt />);
    expect(screen.queryByTestId("first-run-update-prompt")).toBeNull();
  });

  it("yields the bottom-left slot in every pill phase, not just 'available'", () => {
    // A manual check (Connections & Privacy button) can raise the pill
    // before the question is answered, and from there the user can walk it
    // through downloading/ready/error — the two must never stack in ANY of
    // those phases (adversarial review MF1: an `=== "available"` gate
    // survived the original single-phase test).
    const nonIdle = ["available", "downloading", "ready", "error"] as const;
    render(<FirstRunUpdatePrompt />);
    for (const phase of nonIdle) {
      act(() => {
        useUpdateStore.setState({ phase, availableVersion: "9.9.9" });
      });
      expect(
        screen.queryByTestId("first-run-update-prompt"),
        `prompt must hide while the pill shows (phase "${phase}")`,
      ).toBeNull();
    }

    // Pill dismissed → the still-unanswered question returns.
    act(() => {
      useUpdateStore.setState({ phase: "idle", availableVersion: null });
    });
    screen.getByTestId("first-run-update-prompt");
  });
});
