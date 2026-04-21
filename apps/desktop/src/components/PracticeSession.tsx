import { useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";
import PitchDisplay from "./PitchDisplay";
import SessionTimer from "./SessionTimer";
import EndSessionButton from "./EndSessionButton";
import CoachingTipPanel from "./CoachingTipPanel";
import { INSTRUMENTS } from "./InstrumentSelector";

/**
 * Active-session screen: timer + pitch display + coaching tips, with a
 * small header dropdown for mid-session instrument switching.
 */
export default function PracticeSession() {
  const instrumentName = usePracticeStore((s) => s.instrumentName);
  const switchInstrument = usePracticeStore((s) => s.switchInstrument);
  const [menuOpen, setMenuOpen] = useState(false);
  const [switchError, setSwitchError] = useState<string | null>(null);

  const onPickInstrument = async (name: string) => {
    setMenuOpen(false);
    setSwitchError(null);
    if (!instrumentName || name === instrumentName) return;
    try {
      await switchInstrument(name);
    } catch (err) {
      setSwitchError(String(err));
    }
  };

  return (
    <main
      className="flex min-h-screen flex-col bg-gray-900 text-white"
      data-testid="practice-session"
    >
      <header className="flex items-center justify-between border-b border-gray-800 px-6 py-3">
        <div className="relative">
          <button
            type="button"
            onClick={() => setMenuOpen((v) => !v)}
            data-testid="instrument-switch-button"
            aria-expanded={menuOpen}
            aria-haspopup="listbox"
            className="flex items-center gap-1 rounded px-2 py-1 text-sm text-gray-200 hover:bg-gray-800"
          >
            <span>{instrumentName ?? "Unknown instrument"}</span>
            <span aria-hidden="true" className="text-gray-500">
              ▾
            </span>
          </button>
          {menuOpen && (
            <ul
              role="listbox"
              data-testid="instrument-switch-menu"
              className="absolute left-0 top-full z-10 mt-1 max-h-64 w-48 overflow-auto rounded border border-gray-700 bg-gray-900 shadow-lg"
            >
              {INSTRUMENTS.map((inst) => (
                <li key={inst.name}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={inst.name === instrumentName}
                    onClick={() => void onPickInstrument(inst.name)}
                    data-testid={`instrument-switch-option-${inst.name
                      .toLowerCase()
                      .replace(/\s+/g, "-")}`}
                    className={`block w-full px-3 py-2 text-left text-sm hover:bg-gray-800 ${
                      inst.name === instrumentName
                        ? "text-blue-300"
                        : "text-gray-200"
                    }`}
                  >
                    {inst.name}
                  </button>
                </li>
              ))}
            </ul>
          )}
          {switchError && (
            <p className="mt-1 text-xs text-red-400" role="alert">
              {switchError}
            </p>
          )}
        </div>

        <div className="flex items-center gap-4">
          <SessionTimer />
          <EndSessionButton />
        </div>
      </header>

      <div className="flex flex-1 flex-col items-center justify-center gap-8 lg:flex-row lg:gap-12">
        <PitchDisplay />
        <div className="hidden lg:block">
          <CoachingTipPanel />
        </div>
      </div>
    </main>
  );
}
