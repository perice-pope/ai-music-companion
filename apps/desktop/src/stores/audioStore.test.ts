import { describe, it, expect, beforeEach, vi } from "vitest";
import { useAudioStore, frequencyToNote } from "./audioStore";

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

describe("frequencyToNote", () => {
  it("converts A4 (440 Hz) correctly", () => {
    const note = frequencyToNote(440);
    expect(note.name).toBe("A");
    expect(note.octave).toBe(4);
    expect(Math.abs(note.cents_deviation)).toBeLessThan(1);
  });

  it("converts middle C (261.63 Hz) correctly", () => {
    const note = frequencyToNote(261.63);
    expect(note.name).toBe("C");
    expect(note.octave).toBe(4);
    expect(Math.abs(note.cents_deviation)).toBeLessThan(2);
  });

  it("detects sharp pitch", () => {
    // 450 Hz is sharper than A4 (440 Hz)
    const note = frequencyToNote(450);
    expect(note.name).toBe("A");
    expect(note.cents_deviation).toBeGreaterThan(0);
  });

  it("detects flat pitch", () => {
    // 430 Hz is flatter than A4 (440 Hz)
    const note = frequencyToNote(430);
    expect(note.name).toBe("A");
    expect(note.cents_deviation).toBeLessThan(0);
  });
});

describe("audioStore", () => {
  beforeEach(() => {
    localStorageMock.clear();
    vi.clearAllMocks();
    useAudioStore.setState({
      latestEvent: null,
      currentNote: null,
      isListening: false,
      selectedInstrument: null,
    });
  });

  it("starts with null state", () => {
    const state = useAudioStore.getState();
    expect(state.latestEvent).toBeNull();
    expect(state.currentNote).toBeNull();
    expect(state.isListening).toBe(false);
    expect(state.selectedInstrument).toBeNull();
  });

  it("setEvent updates latestEvent and derives note", () => {
    useAudioStore.getState().setEvent({
      pitch_hz: 440,
      confidence: 0.95,
      amplitude: 0.8,
      timestamp_secs: 1.0,
      is_onset: false,
    });

    const state = useAudioStore.getState();
    expect(state.latestEvent).not.toBeNull();
    expect(state.currentNote?.name).toBe("A");
    expect(state.currentNote?.octave).toBe(4);
  });

  it("setEvent with null pitch clears currentNote", () => {
    useAudioStore.getState().setEvent({
      pitch_hz: null,
      confidence: 0.0,
      amplitude: 0.01,
      timestamp_secs: 2.0,
      is_onset: false,
    });

    const state = useAudioStore.getState();
    expect(state.latestEvent).not.toBeNull();
    expect(state.currentNote).toBeNull();
  });

  it("setListening updates state", () => {
    useAudioStore.getState().setListening(true);
    expect(useAudioStore.getState().isListening).toBe(true);

    useAudioStore.getState().setListening(false);
    expect(useAudioStore.getState().isListening).toBe(false);
  });

  it("setInstrument updates selectedInstrument", () => {
    useAudioStore.getState().setInstrument("Trumpet");
    expect(useAudioStore.getState().selectedInstrument).toBe("Trumpet");
  });

  it("setInstrument persists to localStorage", () => {
    useAudioStore.getState().setInstrument("Violin");
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:selected-instrument",
      "Violin",
    );
  });

  it("setInstrument can change selection", () => {
    useAudioStore.getState().setInstrument("Trumpet");
    expect(useAudioStore.getState().selectedInstrument).toBe("Trumpet");

    useAudioStore.getState().setInstrument("Piano");
    expect(useAudioStore.getState().selectedInstrument).toBe("Piano");
  });
});
