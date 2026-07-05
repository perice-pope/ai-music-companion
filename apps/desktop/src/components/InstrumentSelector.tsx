import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAudioStore, type InstrumentInfo } from "../stores/audioStore";
import KeyWheel from "./KeyWheel";
import { usePracticeStore } from "../stores/practiceStore";
import { PRACTICE_MODES } from "../types/brain";

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
  const practiceMode = usePracticeStore((s) => s.practiceMode);
  const setPracticeMode = usePracticeStore((s) => s.setPracticeMode);
  const [startError, setStartError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  // Catalog is fetched from the Rust backend on mount. `null` means
  // "still loading"; `[]` would mean "loaded but empty" which would be
  // an error state — the backend panics rather than returning empty.
  const [instruments, setInstruments] = useState<InstrumentInfo[] | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<InstrumentInfo[]>("list_instruments")
      .then((list) => {
        if (!cancelled) setInstruments(list);
      })
      .catch((err) => {
        if (!cancelled) setCatalogError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const onStart = async () => {
    if (!selectedInstrument) return;
    setStartError(null);
    setStarting(true);
    try {
      const inst = instruments?.find((i) => i.name === selectedInstrument);
      await startSession(selectedInstrument, inst?.vibratoToleranceCents);
    } catch (err) {
      setStartError(String(err));
    } finally {
      setStarting(false);
    }
  };

  const activeMode =
    PRACTICE_MODES.find((m) => m.value === practiceMode) ?? PRACTICE_MODES[1];

  return (
    <section className="w-full max-w-3xl px-4" data-testid="instrument-selector">
      <h2 className="mb-4 text-center text-xl font-semibold text-gray-200">
        Select Your Instrument
      </h2>

      {catalogError && (
        <p
          className="mb-4 text-center text-sm text-red-400"
          role="alert"
          data-testid="catalog-error"
        >
          Failed to load instruments: {catalogError}
        </p>
      )}

      {instruments === null && !catalogError && (
        <p
          className="mb-4 text-center text-sm text-gray-400"
          data-testid="catalog-loading"
        >
          Loading instruments…
        </p>
      )}

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {(instruments ?? []).map((instrument) => {
          const isSelected = selectedInstrument === instrument.name;
          return (
            <button
              key={instrument.name}
              type="button"
              onClick={() => setInstrument(instrument.name, instrument.vibratoToleranceCents)}
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

      {/* Practice mode — three-way segmented control. Persisted across
          launches via the store. The active mode's description is shown
          beneath so the distinction between modes is clear to new users. */}
      <div
        className="mt-6 flex flex-col items-center gap-2"
        data-testid="practice-mode-selector"
      >
        <span className="text-xs uppercase tracking-wider text-gray-500">
          Practice mode
        </span>
        <div
          role="radiogroup"
          aria-label="Practice mode"
          className="inline-flex rounded-full border border-gray-700 bg-gray-800/60 p-1"
        >
          {PRACTICE_MODES.map((mode) => {
            const isActive = mode.value === practiceMode;
            return (
              <button
                key={mode.value}
                type="button"
                role="radio"
                aria-checked={isActive}
                onClick={() => setPracticeMode(mode.value)}
                data-testid={`practice-mode-${mode.value}`}
                className={`rounded-full px-4 py-1 text-sm font-medium transition-colors
                  ${
                    isActive
                      ? "bg-blue-600 text-white shadow"
                      : "text-gray-300 hover:text-white"
                  }`}
              >
                {mode.label}
              </button>
            );
          })}
        </div>
        <p
          className="text-center text-xs text-gray-400"
          data-testid="practice-mode-description"
        >
          {activeMode.description}
        </p>
      </div>

      {/* Start Practice or Practice with Score — enabled only once an instrument is picked. */}
      <div className="mt-6 flex flex-col items-center gap-3">
        <div className="flex gap-3">
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
          <button
            type="button"
            onClick={() => usePracticeStore.setState({ screen: "score-picker" })}
            disabled={!selectedInstrument}
            data-testid="practice-with-score-button"
            className={`rounded-full px-6 py-2 text-base font-semibold transition-colors
              ${
                !selectedInstrument
                  ? "cursor-not-allowed bg-gray-700 text-gray-400"
                  : "bg-purple-600 text-white hover:bg-purple-500"
              }`}
          >
            Practice with Score
          </button>
        </div>
        {startError && (
          <p className="text-sm text-red-400" role="alert" data-testid="start-error">
            {startError}
          </p>
        )}
      </div>
      {/* The 12-key mastery wheel (#256): the Learner Model made visible. */}
      <div className="mt-8 flex justify-center">
        <KeyWheel />
      </div>
    </section>
  );
}
