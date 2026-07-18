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
        note_info: null,
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
        note_info: null,
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
        note_info: null,
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
        note_info: null,
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
        note_info: null,
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
          note_info: null,
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

// #417 rule 0 — the founder's piano session: "the screen blinks on and off
// with the tuner… it needs to be static." The meter is one persistent
// surface: live while a note sounds, HELD (dimmed, frozen reading) when it
// decays, IDLE (dimmer still, same layout) after sustained silence on the
// audio clock. It never swaps back to a bare "Listening..." placeholder
// once anything has sounded.
describe("PitchDisplay — static surface (#417)", () => {
  const event = (t: number) => ({
    pitch_hz: 440,
    confidence: 0.95,
    amplitude: 0.8,
    timestamp_secs: t,
    is_onset: false,
    note_info: null,
  });
  const noteA4 = {
    name: "A",
    octave: 4,
    cents_deviation: 2.3,
    frequency_hz: 440.0,
  };

  it("holds the last reading dimmed when the note decays — no unmount", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: event(1.0),
      currentNote: noteA4,
    });
    render(<PitchDisplay />);
    expect(screen.getByTestId("pitch-display").dataset.surface).toBe("live");

    // The note ends (piano decay): silent events keep arriving.
    act(() => {
      useAudioStore.setState({ latestEvent: event(1.5), currentNote: null });
    });
    const held = screen.getByTestId("pitch-display");
    expect(held.dataset.surface).toBe("held");
    // The reading is FROZEN and visible — letter, Hz, meter all present.
    screen.getByText("A");
    screen.getByText("440 Hz");
    screen.getByTestId("pitch-meter-indicator");
    expect(held.className).toContain("opacity-50");
    // Critically: no "Listening..." swap-out.
    expect(screen.queryByText("Listening...")).toBeNull();
  });

  it("settles to idle after sustained silence on the AUDIO clock", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: event(1.0),
      currentNote: noteA4,
    });
    render(<PitchDisplay />);
    // 3.5s of audio-time silence later (wall clock irrelevant).
    act(() => {
      useAudioStore.setState({ latestEvent: event(4.6), currentNote: null });
    });
    const idle = screen.getByTestId("pitch-display");
    expect(idle.dataset.surface).toBe("idle");
    expect(idle.className).toContain("opacity-30");
    // Same skeleton — the meter never left the DOM.
    screen.getByTestId("pitch-meter-indicator");
  });

  it("wakes instantly from held or idle when a new note sounds", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: event(1.0),
      currentNote: noteA4,
    });
    render(<PitchDisplay />);
    act(() => {
      useAudioStore.setState({ latestEvent: event(4.6), currentNote: null });
    });
    expect(screen.getByTestId("pitch-display").dataset.surface).toBe("idle");
    act(() => {
      useAudioStore.setState({
        latestEvent: event(4.7),
        currentNote: { ...noteA4, name: "C", octave: 5, frequency_hz: 523.3 },
      });
    });
    const live = screen.getByTestId("pitch-display");
    expect(live.dataset.surface).toBe("live");
    expect(live.className).toContain("opacity-100");
    screen.getByText("C");
  });

  it("a new session never inherits a stale held reading", () => {
    useAudioStore.setState({
      isListening: true,
      latestEvent: event(1.0),
      currentNote: noteA4,
    });
    const { rerender } = render(<PitchDisplay />);
    act(() => {
      useAudioStore.setState({
        isListening: false,
        latestEvent: null,
        currentNote: null,
      });
    });
    rerender(<PitchDisplay />);
    screen.getByText("Not listening");
    act(() => {
      useAudioStore.setState({ isListening: true });
    });
    rerender(<PitchDisplay />);
    // Fresh session, nothing sounded yet: calm warm-up copy, no ghost of A4.
    screen.getByText("Listening...");
    expect(screen.queryByText("A")).toBeNull();
  });
});
