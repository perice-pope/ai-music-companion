import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import SessionRecap from "./SessionRecap";
import { usePracticeStore } from "../stores/practiceStore";
import type { SessionRecap as RecapT } from "../types/brain";

/**
 * Offline-first guarantee, exercised at the Face layer:
 *
 * The core loop ends at the session recap. The Rust core always produces a
 * recap, falling back to a fully on-device, fact-grounded recap when no LLM /
 * HttpClient is reachable (see `crates/brain/src/coaching.rs::fallback_recap`).
 * This test stands in for that offline path on the frontend: given a
 * fallback-shaped recap (the kind the core emits with the network off), the
 * recap screen renders the complete practice → recap result WITHOUT any
 * network dependency.
 *
 * We assert nothing reaches for the network: `invoke` (the IPC/network bridge)
 * is never called while rendering the recap, and Supabase is never touched.
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// If anything in the recap path tried to sync, it would hit Supabase. Fail
// loudly if it does.
const supabaseTrap = vi.fn(() => {
  throw new Error("recap path must not touch Supabase");
});
vi.mock("../lib/supabase", () => ({
  supabase: { from: supabaseTrap, auth: { onAuthStateChange: supabaseTrap } },
}));

/**
 * A recap shaped like the on-device fallback the Rust core emits with no LLM
 * available: generic-but-honest encouragement, every field present, grounded
 * only in locally measured facts (duration, phrase count, instrument).
 */
function offlineFallbackRecap(): RecapT {
  return {
    overall_assessment:
      "You completed a 10 minute practice session on trumpet. " +
      "That's excellent dedication! You played 8 phrases and showed " +
      "consistent engagement.",
    strengths: [
      "Consistent practice and focus.",
      "You showed up and played — that's what matters most.",
    ],
    areas_to_improve: [
      "Every session builds on the last one.",
      "Keep recording yourself to track progress.",
    ],
    next_session_suggestions: [
      "Work on the passages that felt most challenging.",
      "Try breaking difficult sections into smaller chunks.",
    ],
    duration_secs: 600,
    phrase_count: 8,
    instrument: "Trumpet",
  };
}

describe("offline practice → recap path", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usePracticeStore.setState({
      screen: "recap",
      status: "idle",
      sessionId: null,
      instrumentName: null,
      segmentId: null,
      startedAtMs: null,
      elapsedSecs: 0,
      phrases: [],
      tipQueue: [],
      recap: offlineFallbackRecap(),
      recapError: null,
    });
  });

  it("renders the full recap with no network dependency (offline fallback)", () => {
    render(<SessionRecap />);

    // The complete recap is present on screen… (getBy* throws if absent)
    expect(
      screen.getByText(/completed a 10 minute practice session/i),
    ).toBeTruthy();
    expect(screen.getByTestId("recap-strengths")).toBeTruthy();
    expect(screen.getByTestId("recap-areas")).toBeTruthy();

    // …and getting here required no network: no IPC/network bridge call, no
    // Supabase access.
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(supabaseTrap).not.toHaveBeenCalled();
  });
});
