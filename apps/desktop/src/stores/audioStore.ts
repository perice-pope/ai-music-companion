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

interface AudioState {
  /** Most recent audio event from the backend. */
  latestEvent: AudioEvent | null;
  /** Derived note information. */
  currentNote: NoteInfo | null;
  /** Whether the audio capture is active. */
  isListening: boolean;

  setEvent: (event: AudioEvent) => void;
  setListening: (listening: boolean) => void;
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

export const useAudioStore = create<AudioState>((set) => ({
  latestEvent: null,
  currentNote: null,
  isListening: false,

  setEvent: (event: AudioEvent) => {
    const currentNote =
      event.pitch_hz !== null ? frequencyToNote(event.pitch_hz) : null;
    set({ latestEvent: event, currentNote });
  },

  setListening: (listening: boolean) => set({ isListening: listening }),
}));
