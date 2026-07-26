import { create } from "zustand";
import type { InstrumentInfo } from "../types/brain";

/** Re-exported so existing callers can keep `import ... from "./audioStore"`. */
export type { InstrumentInfo };

/** Mirrors the Rust NoteInfo struct sent over Tauri IPC. */
export interface RustNoteInfo {
  midi_note: number;
  note_name: string;
  octave: number;
  cents_deviation: number;
}

/** Mirrors the Rust AudioEvent struct sent over Tauri IPC. */
export interface AudioEvent {
  pitch_hz: number | null;
  confidence: number;
  amplitude: number;
  timestamp_secs: number;
  is_onset: boolean;
  note_info: RustNoteInfo | null;
}

/** Derived note info for the UI. */
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
  /** Currently selected instrument name, or null if none. */
  selectedInstrument: string | null;
  /** Vibrato tolerance in cents for the current instrument. */
  instrumentVibratoToleranceCents: number;

  setEvent: (event: AudioEvent) => void;
  setListening: (listening: boolean) => void;
  setInstrument: (name: string, vibratoToleranceCents: number) => void;
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
  instrumentVibratoToleranceCents: 15.0,

  setEvent: (event: AudioEvent) => {
    // Convert pre-computed Rust note info to UI format.
    const currentNote =
      event.note_info !== null && event.pitch_hz !== null
        ? {
            name: event.note_info.note_name,
            octave: event.note_info.octave,
            cents_deviation: event.note_info.cents_deviation,
            frequency_hz: Math.round(event.pitch_hz * 10) / 10,
          }
        : null;
    // Flip `isListening` on first event — an arriving event is proof
    // that the backend pipeline is hot. This keeps the flag
    // self-regulating: if the mic never opened (start_practice_session
    // logged the failure but continued the session), no events arrive
    // and `isListening` stays false, so `PitchDisplay` shows its
    // idle copy instead of lying about a dead mic. `setListening(false)`
    // on session end clears it back.
    set({ latestEvent: event, currentNote, isListening: true });
  },

  setListening: (listening: boolean) => set({ isListening: listening }),

  setInstrument: (name: string, vibratoToleranceCents: number) => {
    try {
      localStorage.setItem(INSTRUMENT_STORAGE_KEY, name);
    } catch {
      // localStorage may be unavailable in some environments
    }
    set({
      selectedInstrument: name,
      instrumentVibratoToleranceCents: vibratoToleranceCents,
    });
  },
}));
