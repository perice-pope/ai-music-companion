import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
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
    expect(screen.getByText("Not listening")).toBeDefined();
  });

  it("shows listening state when active but no data", () => {
    useAudioStore.setState({ isListening: true });
    render(<PitchDisplay />);
    expect(screen.getByText("Listening...")).toBeDefined();
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
    expect(screen.getByText("A")).toBeDefined();
    expect(screen.getByText("4")).toBeDefined();
    expect(screen.getByText("440 Hz")).toBeDefined();
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
    expect(screen.getByText("+19.6 cents")).toBeDefined();
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
    const indicator = screen.getByTestId("pitch-meter-indicator");
    expect(indicator).toBeDefined();
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
    expect(screen.getByText("confidence: 92%")).toBeDefined();
  });
});
