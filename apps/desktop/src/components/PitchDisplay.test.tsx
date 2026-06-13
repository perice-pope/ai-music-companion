import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import PitchDisplay from "./PitchDisplay";
import { useAudioStore } from "../stores/audioStore";

describe("PitchDisplay", () => {
  beforeEach(() => {
    useAudioStore.setState({
      latestEvent: null,
      currentNote: null,
      isListening: false,
    });
  });

  it("shows not listening when inactive", () => {
    render(<PitchDisplay />);
    screen.getByText("Not listening");
  });

  it("shows listening state when active but no data", () => {
    useAudioStore.setState({ isListening: true });
    render(<PitchDisplay />);
    screen.getByText("Listening...");
  });

  it("displays note name when pitch is detected", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: {
        pitch_hz: 440,
        confidence: 0.95,
        amplitude: 0.8,
        timestamp_secs: 1.0,
        is_onset: false,
      },
      currentNote: {
        name: "A",
        octave: 4,
        cents_deviation: 2.3,
        frequency_hz: 440.0,
      },
    });

    render(<PitchDisplay />);
    screen.getByText("A");
    screen.getByText("4");
    screen.getByText("440 Hz");
  });

  it("displays cents deviation", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: {
        pitch_hz: 445,
        confidence: 0.9,
        amplitude: 0.7,
        timestamp_secs: 1.0,
        is_onset: false,
      },
      currentNote: {
        name: "A",
        octave: 4,
        cents_deviation: 19.6,
        frequency_hz: 445.0,
      },
    });

    render(<PitchDisplay />);
    screen.getByText("+19.6 cents");
  });

  it("has a pitch meter indicator", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: {
        pitch_hz: 440,
        confidence: 0.95,
        amplitude: 0.8,
        timestamp_secs: 1.0,
        is_onset: false,
      },
      currentNote: {
        name: "A",
        octave: 4,
        cents_deviation: 0,
        frequency_hz: 440.0,
      },
    });

    render(<PitchDisplay />);
    screen.getByTestId("pitch-meter-indicator");
  });

  it("shows confidence percentage", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: {
        pitch_hz: 440,
        confidence: 0.92,
        amplitude: 0.8,
        timestamp_secs: 1.0,
        is_onset: false,
      },
      currentNote: {
        name: "A",
        octave: 4,
        cents_deviation: 0,
        frequency_hz: 440.0,
      },
    });

    render(<PitchDisplay />);
    screen.getByText("confidence: 92%");
  });

  it("smooths the displayed cents so it doesn't snap on every event (#187)", async () => {
    // Baseline reading at 0 cents.
    useAudioStore.setState({
      isListening: true,
      latestEvent: {
        pitch_hz: 440,
        confidence: 0.95,
        amplitude: 0.8,
        timestamp_secs: 1.0,
        is_onset: false,
      },
      currentNote: {
        name: "A",
        octave: 4,
        cents_deviation: 0,
        frequency_hz: 440.0,
      },
    });
    render(<PitchDisplay />);
    await screen.findByText("0.0 cents");

    // A sudden jump to +40 cents must NOT snap the read-out straight to 40 —
    // the EMA damps it to an intermediate value, so the number stays readable.
    act(() => {
      useAudioStore.setState({
        latestEvent: {
          pitch_hz: 450,
          confidence: 0.95,
          amplitude: 0.8,
          timestamp_secs: 1.05,
          is_onset: false,
        },
        currentNote: {
          name: "A",
          octave: 4,
          cents_deviation: 40,
          frequency_hz: 450.0,
        },
      });
    });

    await waitFor(() => {
      const text = screen.getByText(/cents$/).textContent ?? "";
      const shown = parseFloat(text.replace(/[^0-9.-]/g, ""));
      expect(shown).toBeGreaterThan(0);
      expect(shown).toBeLessThan(40);
    });
  });
});
