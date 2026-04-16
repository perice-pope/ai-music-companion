import { create } from "zustand";

/** Mirrors the Rust AudioEvent struct sent over Tauri IPC. */
export interface AudioEvent {
  pitch_hz: number | null;
  confidence: number;
  amplitude: number;
  timestamp_secs: number;
  is_onset: boolean;
}

/** Derived note info computed from frequency. */
export interface NoteInfo {
  name: string;
  octave: number;
  cents_deviation: number;
  frequency_hz: number;
}

/** An instrument available for selection. */
export interface InstrumentInfo {
  name: string;
  family: string;
  freqMinHz: number;
  freqMaxHz: number;
  emoji: string;
}

interface AudioState {
  /** Most recent audio event from the backend. */
  latestEvent: AudioEvent | null;
  /** Derived note information. */
  currentNote: NoteInfo | null;
  /** Whether the audio capture is active. */
  isListening: boolean;
  /** Currently selected instrument name, or null if none. */
  selectedInstrument: string | null;

  setEvent: (event: AudioEvent) => void;
  setListening: (listening: boolean) => void;
  setInstrument: (name: string) => void;
}

const NOTE_NAMES = [
  "C",
  "C#",
  "D",
  "D#",
  "E",
  "F",
  "F#",
  "G",
  "G#",
  "A",
  "A#",
  "B",
] as const;

/** Convert a frequency in Hz to the nearest note name, octave, and cents deviation. */
export function frequencyToNote(hz: number): NoteInfo {
  if (!Number.isFinite(hz) || hz <= 0) {
    throw new RangeError("frequencyToNote expects a finite, positive frequency");
  }
  // A4 = 440 Hz = MIDI note 69
  const semitones = 12.0 * Math.log2(hz / 440.0);
  const midiNote = Math.round(semitones) + 69;
  const centsDeviation = (semitones - (midiNote - 69)) * 100;

  const noteIndex = ((midiNote % 12) + 12) % 12;
  const octave = Math.floor(midiNote / 12) - 1;

  return {
    name: NOTE_NAMES[noteIndex],
    octave,
    cents_deviation: Math.round(centsDeviation * 10) / 10,
    frequency_hz: Math.round(hz * 10) / 10,
  };
}

/** Key used to persist the selected instrument in localStorage. */
const INSTRUMENT_STORAGE_KEY = "ai-music-companion:selected-instrument";

/** Load previously selected instrument from localStorage. */
function loadSavedInstrument(): string | null {
  try {
    return localStorage.getItem(INSTRUMENT_STORAGE_KEY);
  } catch {
    return null;
  }
}

export const useAudioStore = create<AudioState>((set) => ({
  latestEvent: null,
  currentNote: null,
  isListening: false,
  selectedInstrument: loadSavedInstrument(),

  setEvent: (event: AudioEvent) => {
    const hz = event.pitch_hz;
    const currentNote =
      hz !== null && Number.isFinite(hz) && hz > 0
        ? frequencyToNote(hz)
        : null;
    set({ latestEvent: event, currentNote });
  },

  setListening: (listening: boolean) => set({ isListening: listening }),

  setInstrument: (name: string) => {
    try {
      localStorage.setItem(INSTRUMENT_STORAGE_KEY, name);
    } catch {
      // localStorage may be unavailable in some environments
    }
    set({ selectedInstrument: name });
  },
}));
