import { describe, it, expect, beforeEach, vi } from "vitest";
import { useAudioStore } from "./audioStore";

// Mock localStorage
const localStorageMock = (() => {
  let store: Record<string, string> = {};
  return {
    getItem: vi.fn((key: string) => store[key] ?? null),
    setItem: vi.fn((key: string, value: string) => {
      store[key] = value;
    }),
    removeItem: vi.fn((key: string) => {
      delete store[key];
    }),
    clear: vi.fn(() => {
      store = {};
    }),
    get length() {
      return Object.keys(store).length;
    },
    key: vi.fn((_index: number) => null),
  };
})();

Object.defineProperty(window, "localStorage", { value: localStorageMock });

describe("audioStore", () => {
  beforeEach(() => {
    localStorageMock.clear();
    vi.clearAllMocks();
    useAudioStore.setState({
      latestEvent: null,
      currentNote: null,
      isListening: false,
      selectedInstrument: null,
      instrumentVibratoToleranceCents: 15.0,
    });
  });

  it("starts with null state", () => {
    const state = useAudioStore.getState();
    expect(state.latestEvent).toBeNull();
    expect(state.currentNote).toBeNull();
    expect(state.isListening).toBe(false);
    expect(state.selectedInstrument).toBeNull();
  });

  it("setEvent derives the note from Rust's note_info, never from TS math", () => {
    // Rust is the sole authority on note naming (CLAUDE.md: no business
    // logic in the frontend). This event's note_info deliberately
    // DISAGREES with everything TS could re-derive it from: 440 Hz is
    // A4 ±0¢ (kills pitch-based math), midi 70 maps to "A#"/octave 4
    // through a sharp note table and floor(midi/12)-1 (kills a
    // resurrected NOTE_NAMES lookup — "Bb" is not in a sharp table, and
    // Rust owns enharmonic spelling). Any recomputation goes red here.
    const event = {
      pitch_hz: 440,
      confidence: 0.95,
      amplitude: 0.8,
      timestamp_secs: 1.0,
      is_onset: true,
      note_info: {
        midi_note: 70,
        note_name: "Bb",
        octave: 3,
        cents_deviation: -12.5,
      },
    };
    useAudioStore.getState().setEvent(event);

    const state = useAudioStore.getState();
    expect(state.latestEvent).toEqual(event);
    expect(state.currentNote).toEqual({
      name: "Bb",
      octave: 3,
      cents_deviation: -12.5,
      frequency_hz: 440,
    });
  });

  it("setEvent rounds frequency_hz to one decimal for display", () => {
    useAudioStore.getState().setEvent({
      pitch_hz: 261.6256,
      confidence: 0.9,
      amplitude: 0.5,
      timestamp_secs: 1.5,
      is_onset: false,
      note_info: {
        midi_note: 60,
        note_name: "C",
        octave: 4,
        cents_deviation: 0,
      },
    });

    expect(useAudioStore.getState().currentNote?.frequency_hz).toBe(261.6);
  });

  it("setEvent with null pitch clears currentNote but keeps listening", () => {
    useAudioStore.getState().setEvent({
      pitch_hz: null,
      confidence: 0.0,
      amplitude: 0.01,
      timestamp_secs: 2.0,
      is_onset: false,
      note_info: null,
    });

    const state = useAudioStore.getState();
    expect(state.latestEvent).not.toBeNull();
    expect(state.currentNote).toBeNull();
    // The ears silence gate streams null-pitch events through every quiet
    // moment of a live session; they still prove the pipeline is hot.
    // Only session end (setListening(false)) may drop the flag.
    expect(state.isListening).toBe(true);
  });

  it("setEvent needs BOTH pitch_hz and note_info before naming a note", () => {
    // pitch_hz without note_info is legitimate (Rust omits note_info for
    // out-of-MIDI-range frequencies); note_info without pitch_hz shouldn't
    // happen. Either way the display contract is the same: no half-built
    // note — a name with no frequency (or vice versa) must render nothing.
    useAudioStore.getState().setEvent({
      pitch_hz: null,
      confidence: 0.4,
      amplitude: 0.2,
      timestamp_secs: 3.0,
      is_onset: false,
      note_info: {
        midi_note: 69,
        note_name: "A",
        octave: 4,
        cents_deviation: 0,
      },
    });
    expect(useAudioStore.getState().currentNote).toBeNull();

    useAudioStore.getState().setEvent({
      pitch_hz: 440,
      confidence: 0.4,
      amplitude: 0.2,
      timestamp_secs: 3.1,
      is_onset: false,
      note_info: null,
    });
    expect(useAudioStore.getState().currentNote).toBeNull();
  });

  it("setListening updates state", () => {
    useAudioStore.getState().setListening(true);
    expect(useAudioStore.getState().isListening).toBe(true);

    useAudioStore.getState().setListening(false);
    expect(useAudioStore.getState().isListening).toBe(false);
  });

  it("setEvent flips isListening to true — arriving events mean the mic is hot", () => {
    // Invariant: the store shouldn't assume the mic opened just because
    // the practice session started. It's the first event that proves
    // the backend pipeline is live. Failing to open the mic on Rust's
    // side means no events come, which means PitchDisplay must NOT
    // flip out of its idle state.
    expect(useAudioStore.getState().isListening).toBe(false);
    useAudioStore.getState().setEvent({
      pitch_hz: 440,
      confidence: 0.95,
      amplitude: 0.8,
      timestamp_secs: 1.0,
      is_onset: false,
      note_info: {
        midi_note: 69,
        note_name: "A",
        octave: 4,
        cents_deviation: 0,
      },
    });
    expect(useAudioStore.getState().isListening).toBe(true);
  });

  it("setInstrument updates selectedInstrument", () => {
    useAudioStore.getState().setInstrument("Trumpet", 20.0);
    expect(useAudioStore.getState().selectedInstrument).toBe("Trumpet");
    expect(useAudioStore.getState().instrumentVibratoToleranceCents).toBe(20.0);
  });

  it("setInstrument persists to localStorage", () => {
    useAudioStore.getState().setInstrument("Violin", 15.0);
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:selected-instrument",
      "Violin",
    );
  });

  it("setInstrument can change selection", () => {
    useAudioStore.getState().setInstrument("Trumpet", 20.0);
    expect(useAudioStore.getState().selectedInstrument).toBe("Trumpet");
    expect(useAudioStore.getState().instrumentVibratoToleranceCents).toBe(20.0);

    useAudioStore.getState().setInstrument("Piano", 10.0);
    expect(useAudioStore.getState().selectedInstrument).toBe("Piano");
    expect(useAudioStore.getState().instrumentVibratoToleranceCents).toBe(10.0);
  });
});
