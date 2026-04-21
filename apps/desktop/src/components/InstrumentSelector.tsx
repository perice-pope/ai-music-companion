import { useState } from "react";
import { useAudioStore, type InstrumentInfo } from "../stores/audioStore";
import { usePracticeStore } from "../stores/practiceStore";

/** All available instruments (hardcoded for Phase 1; will come from profiles later). */
export const INSTRUMENTS: InstrumentInfo[] = [
  { name: "Trumpet", family: "Brass", freqMinHz: 165, freqMaxHz: 1047, emoji: "\uD83C\uDFBA" },
  { name: "Trombone", family: "Brass", freqMinHz: 58, freqMaxHz: 587, emoji: "\uD83C\uDFB5" },
  { name: "French Horn", family: "Brass", freqMinHz: 87, freqMaxHz: 880, emoji: "\uD83D\uDCEF" },
  { name: "Violin", family: "Strings", freqMinHz: 196, freqMaxHz: 3136, emoji: "\uD83C\uDFBB" },
  { name: "Cello", family: "Strings", freqMinHz: 65, freqMaxHz: 988, emoji: "\uD83C\uDFB6" },
  { name: "Flute", family: "Woodwind", freqMinHz: 262, freqMaxHz: 2093, emoji: "\uD83C\uDFB6" },
  { name: "Clarinet", family: "Woodwind", freqMinHz: 147, freqMaxHz: 1568, emoji: "\uD83C\uDFB5" },
  { name: "Voice", family: "Voice", freqMinHz: 82, freqMaxHz: 1047, emoji: "\uD83C\uDFA4" },
  { name: "Piano", family: "Keyboard", freqMinHz: 28, freqMaxHz: 4186, emoji: "\uD83C\uDFB9" },
];

/** Format a frequency range for display (e.g., "165 – 1047 Hz"). */
function formatRange(min: number, max: number): string {
  return `${min} \u2013 ${max} Hz`;
}

/** Color for the family badge. */
function familyColor(family: string): string {
  switch (family) {
    case "Brass":
      return "bg-yellow-900/40 text-yellow-300";
    case "Strings":
      return "bg-purple-900/40 text-purple-300";
    case "Woodwind":
      return "bg-green-900/40 text-green-300";
    case "Voice":
      return "bg-blue-900/40 text-blue-300";
    case "Keyboard":
      return "bg-red-900/40 text-red-300";
    default:
      return "bg-gray-700 text-gray-300";
  }
}

export default function InstrumentSelector() {
  const selectedInstrument = useAudioStore((s) => s.selectedInstrument);
  const setInstrument = useAudioStore((s) => s.setInstrument);
  const startSession = usePracticeStore((s) => s.startSession);
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  const onStart = async () => {
    if (!selectedInstrument) return;
    setStartError(null);
    setStarting(true);
    try {
      await startSession(selectedInstrument);
    } catch (err) {
      setStartError(String(err));
    } finally {
      setStarting(false);
    }
  };

  return (
    <section className="w-full max-w-3xl px-4" data-testid="instrument-selector">
      <h2 className="mb-4 text-center text-xl font-semibold text-gray-200">
        Select Your Instrument
      </h2>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {INSTRUMENTS.map((instrument) => {
          const isSelected = selectedInstrument === instrument.name;
          return (
            <button
              key={instrument.name}
              type="button"
              onClick={() => setInstrument(instrument.name)}
              data-testid={`instrument-card-${instrument.name.toLowerCase().replace(/\s+/g, "-")}`}
              className={`
                relative flex flex-col items-center gap-2 rounded-xl border-2 p-4
                transition-all duration-200 ease-in-out
                hover:scale-[1.03] hover:shadow-lg
                focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-gray-900
                ${
                  isSelected
                    ? "border-blue-500 bg-blue-950/50 shadow-blue-500/20 shadow-lg"
                    : "border-gray-700 bg-gray-800/60 hover:border-gray-500"
                }
              `}
              aria-pressed={isSelected}
            >
              {/* Last-used indicator */}
              {isSelected && (
                <span
                  className="absolute right-2 top-2 text-xs text-blue-400"
                  data-testid="selected-indicator"
                >
                  &#10003;
                </span>
              )}

              {/* Emoji icon */}
              <span className="text-3xl" role="img" aria-label={instrument.name}>
                {instrument.emoji}
              </span>

              {/* Instrument name */}
              <span className="text-sm font-medium text-gray-100">
                {instrument.name}
              </span>

              {/* Family badge */}
              <span
                className={`rounded-full px-2 py-0.5 text-[10px] font-medium uppercase tracking-wider ${familyColor(instrument.family)}`}
              >
                {instrument.family}
              </span>

              {/* Frequency range */}
              <span className="text-[10px] text-gray-500">
                {formatRange(instrument.freqMinHz, instrument.freqMaxHz)}
              </span>
            </button>
          );
        })}
      </div>

      {/* Start Practice — enabled only once an instrument is picked. */}
      <div className="mt-6 flex flex-col items-center gap-2">
        <button
          type="button"
          onClick={() => void onStart()}
          disabled={!selectedInstrument || starting}
          data-testid="start-practice-button"
          className={`rounded-full px-6 py-2 text-base font-semibold transition-colors
            ${
              !selectedInstrument || starting
                ? "cursor-not-allowed bg-gray-700 text-gray-400"
                : "bg-blue-600 text-white hover:bg-blue-500"
            }`}
        >
          {starting ? "Starting…" : "Start Practice"}
        </button>
        {startError && (
          <p className="text-sm text-red-400" role="alert" data-testid="start-error">
            {startError}
          </p>
        )}
      </div>
    </section>
  );
}
