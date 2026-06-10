import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import SessionRecap from "./SessionRecap";
import { usePracticeStore } from "../stores/practiceStore";
import type { SessionRecap as RecapT } from "../types/brain";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function fullRecap(overrides: Partial<RecapT> = {}): RecapT {
  return {
    overall_assessment: "You had a solid 15 minutes on trumpet.",
    strengths: ["Consistent tone.", "Steady tempo."],
    areas_to_improve: [
      "Intonation drifted sharp on high C.",
      "Rests sometimes feel cut short.",
    ],
    next_session_suggestions: [
      "Long tones with a drone.",
      "Slow tonguing exercises.",
    ],
    duration_secs: 900,
    phrase_count: 24,
    instrument: "Trumpet",
    ...overrides,
  };
}

function seedRecap(recap: RecapT | null, error: string | null = null) {
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
    recap,
    recapError: error,
  });
}

describe("SessionRecap", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders strengths BEFORE areas-to-improve in DOM order", () => {
    seedRecap(fullRecap());
    render(<SessionRecap />);

    const strengths = screen.getByTestId("recap-strengths");
    const areas = screen.getByTestId("recap-areas");

    // Use compareDocumentPosition to assert DOM order, not visual
    // position. Math: DOCUMENT_POSITION_FOLLOWING (4) means `areas`
    // comes after `strengths`.
    const following =
      strengths.compareDocumentPosition(areas) &
      Node.DOCUMENT_POSITION_FOLLOWING;
    expect(following).toBeTruthy();
  });

  it("renders the overall assessment and all three sections", () => {
    seedRecap(fullRecap());
    render(<SessionRecap />);
    expect(screen.getByTestId("recap-assessment").textContent).toContain(
      "solid 15 minutes",
    );
    screen.getByText("Consistent tone.");
    screen.getByText("Intonation drifted sharp on high C.");
    screen.getByText("Long tones with a drone.");
  });

  it("shows the tone read-out only when the recap carries a tone aggregate", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no tone panel.
    expect(screen.queryByTestId("recap-tone")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          tone: {
            brightness: 0.6,
            warmth: 0.5,
            air_noise: 0.2,
            core_clarity: 0.8,
            vibrato_quality: 0.55,
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    expect(screen.getByTestId("recap-tone")).toBeTruthy();
    screen.getByText("Brightness");
  });

  it("shows the detected key only when the recap carries one", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no key line.
    expect(screen.queryByTestId("recap-key")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          key: {
            tonic: 7,
            mode: "mixolydian",
            confidence: 0.82,
            margin: 0.1,
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    expect(screen.getByTestId("recap-key").textContent).toContain(
      "G Mixolydian",
    );
  });

  it("shows the intonation read-out only when the recap carries one", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no intonation line.
    expect(screen.queryByTestId("recap-intonation")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          intonation: {
            note_count: 24,
            mean_cents: 8,
            mean_abs_cents: 12,
            in_tune_ratio: 0.75,
            tendencies: [{ semitones_from_tonic: 4, mean_cents: 14, count: 6 }],
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    const line = screen.getByTestId("recap-intonation").textContent ?? "";
    expect(line).toContain("tends sharp");
    expect(line).toContain("major 3rd");
  });

  it("shows the groove read-out only when the recap carries one", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no feel line.
    expect(screen.queryByTestId("recap-groove")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          groove: {
            tempo_bpm: 92,
            swing_ratio: 1.6,
            mean_ioi_secs: 0.4,
            timing_consistency: 0.95,
            onset_count: 40,
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    const line = screen.getByTestId("recap-groove").textContent ?? "";
    expect(line).toContain("92 BPM");
    expect(line).toContain("swung");
    expect(line).toContain("steady");
  });

  it("shows cross-genre connections only when the recap carries grounded ones", () => {
    // No connections → no panel (cold start / thin signal / silence).
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    expect(screen.queryByTestId("recap-connections")).toBeNull();

    // Explicit empty array → still nothing (honest about "no grounded link").
    seedRecap(fullRecap({ connections: [] }));
    rerender(<SessionRecap />);
    expect(screen.queryByTestId("recap-connections")).toBeNull();

    // Grounded, hedged connection present → the quiet "In your world" panel.
    seedRecap(
      fullRecap({
        connections: [
          "the way you're laying back on the beat has the same pocket as a lot of the soul you love",
        ],
      }),
    );
    rerender(<SessionRecap />);
    const panel = screen.getByTestId("recap-connections");
    expect(panel).toBeTruthy();
    expect(panel.textContent).toContain("the same pocket");
  });

  it("empty-state recap (zero phrases) hides bullet sections but keeps buttons", () => {
    seedRecap(
      fullRecap({
        overall_assessment:
          "Looks like you didn't get to play this time — come back when you're ready.",
        strengths: [],
        areas_to_improve: [],
        next_session_suggestions: ["Just having the app running can help."],
        phrase_count: 0,
        duration_secs: 0,
      }),
    );
    render(<SessionRecap />);

    expect(screen.queryByTestId("recap-strengths")).toBeNull();
    expect(screen.queryByTestId("recap-areas")).toBeNull();
    screen.getByTestId("recap-next");
    // And crucially we still offer the user a way forward.
    screen.getByTestId("recap-practice-again");
    screen.getByTestId("recap-done");
    // data-variant reflects the mode so styling can differ later.
    expect(
      screen.getByTestId("session-recap").getAttribute("data-variant"),
    ).toBe("empty");
  });

  it("renders the fallback copy when recapError is set", () => {
    seedRecap(null, "llm timeout");
    render(<SessionRecap />);
    expect(
      screen.getByTestId("session-recap").getAttribute("data-variant"),
    ).toBe("error");
    // No strengths/areas sections in the error variant.
    expect(screen.queryByTestId("recap-strengths")).toBeNull();
    expect(screen.queryByTestId("recap-areas")).toBeNull();
    screen.getByText(/trouble generating your recap/i);
  });

  it("Practice again / Done both return to selector", () => {
    seedRecap(null, "llm timeout");
    const { rerender } = render(<SessionRecap />);

    fireEvent.click(screen.getByTestId("recap-practice-again"));
    expect(usePracticeStore.getState().screen).toBe("selector");
    expect(usePracticeStore.getState().recap).toBeNull();
    expect(usePracticeStore.getState().recapError).toBeNull();

    // Re-seed + test the Done button path independently.
    seedRecap(fullRecap());
    rerender(<SessionRecap />);
    fireEvent.click(screen.getByTestId("recap-done"));
    expect(usePracticeStore.getState().screen).toBe("selector");
  });
});
