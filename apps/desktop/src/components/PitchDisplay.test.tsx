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

  // Regression: v2 §4a and §8 — no per-note green/yellow/red as primary feedback.
  // The note name and cents text must stay neutral regardless of deviation.
  // Coaching judgment lives at the phrase layer (CoachingTipPanel, SessionRecap).
  // The continuous tuning-meter indicator keeps its color as a needle-style
  // tuning aid, and that exception is asserted explicitly below.
  describe("no per-note verdict coloring (coach, don't judge)", () => {
    const deviations = [
      { label: "in tune", cents: 0 },
      { label: "slightly sharp", cents: 8 },
      { label: "noticeably flat", cents: -19 },
      { label: "badly sharp", cents: 40 },
    ];

    for (const { label, cents } of deviations) {
      it(`renders note name with neutral color when ${label} (${cents}c)`, () => {
        useAudioStore.setState({
          isListening: true,
          latestEvent: {
            pitch_hz: 440,
            confidence: 0.9,
            amplitude: 0.8,
            timestamp_secs: 1.0,
            is_onset: false,
          },
          currentNote: {
            name: "A",
            octave: 4,
            cents_deviation: cents,
            frequency_hz: 440.0,
          },
        });

        render(<PitchDisplay />);
        const noteName = screen.getByTestId("pitch-note-name");
        const centsText = screen.getByTestId("pitch-cents-text");

        // The note name and cents readout must not carry a verdict color.
        for (const el of [noteName, centsText]) {
          expect(el.className).not.toMatch(/text-(green|yellow|red)-/);
        }
      });
    }

    it("keeps color on the tuning-meter indicator (tuning aid, not a verdict)", () => {
      useAudioStore.setState({
        isListening: true,
        latestEvent: {
          pitch_hz: 445,
          confidence: 0.9,
          amplitude: 0.8,
          timestamp_secs: 1.0,
          is_onset: false,
        },
        currentNote: {
          name: "A",
          octave: 4,
          cents_deviation: 40,
          frequency_hz: 445.0,
        },
      });

      render(<PitchDisplay />);
      const indicator = screen.getByTestId("pitch-meter-indicator");
      // The meter indicator is a needle-style tuning aid; color here IS the signal.
      expect(indicator.className).toMatch(/bg-(green|yellow|red)-/);
    });
  });
});
