import { usePracticeStore } from "../stores/practiceStore";
import InstrumentSelector from "./InstrumentSelector";
import ScorePicker from "./ScorePicker";
import PracticeSession from "./PracticeSession";
import SessionRecap from "./SessionRecap";
import History from "../pages/History";

/**
 * Top-level router for the free-play flow.
 *
 * Three screens — selector, session, recap — switched on the
 * `screen` enum in the practice store. See design doc §2 for the
 * rationale (Zustand enum instead of react-router for three states).
 */
export default function PracticeShell() {
  const screen = usePracticeStore((s) => s.screen);

  switch (screen) {
    case "session":
      return <PracticeSession />;
    case "recap":
      return <SessionRecap />;
    case "history":
      return <History />;
    case "score-picker":
      return <ScorePicker />;
    case "selector":
    default:
      return (
        <main
          className="flex min-h-screen flex-col items-center justify-center gap-8 bg-gray-900 text-white"
          data-testid="practice-shell-selector"
        >
          <div className="text-center">
            <h1 className="text-4xl font-bold">AI Music Companion</h1>
            <p className="mt-2 text-sm text-gray-400">Free Play</p>
          </div>
          <InstrumentSelector />
        </main>
      );
  }
}
