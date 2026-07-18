import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { usePracticeStore } from "../stores/practiceStore";
import type { InstrumentInfo } from "../stores/audioStore";
import { PRACTICE_MODES, type PracticeMode } from "../types/brain";
import PitchDisplay from "./PitchDisplay";
import SessionTimer from "./SessionTimer";
import EndSessionButton from "./EndSessionButton";
import AccompanimentToggle from "./AccompanimentToggle";
import PocketControl from "./PocketControl";
import PerceptionPanel from "./PerceptionPanel";
import CoachingTipPanel from "./CoachingTipPanel";
import RevealCard from "./RevealCard";
import LessonPanel from "./LessonPanel";
import ExplorePanel from "./ExplorePanel";
import LiftLickButton from "./LiftLickButton";
import OpenersPanel from "./OpenersPanel";
import ChordLane from "./ChordLane";
import LiftProgressionButton from "./LiftProgressionButton";
import ScoreView from "./ScoreView";
import VerdictStrip from "./VerdictStrip";
import ScorePhraseCard from "./ScorePhraseCard";

/**
 * Active-session screen: timer + pitch display + coaching tips, with a
 * small header dropdown for mid-session instrument switching.
 *
 * When a score is loaded, the live sheet music (with a following cursor)
 * takes the main stage and the pitch meter moves to a sidebar.
 */
export default function PracticeSession() {
  const instrumentName = usePracticeStore((s) => s.instrumentName);
  const switchInstrument = usePracticeStore((s) => s.switchInstrument);
  const practiceMode = usePracticeStore((s) => s.practiceMode);
  const setPracticeMode = usePracticeStore((s) => s.setPracticeMode);
  const activeScoreXml = usePracticeStore((s) => s.activeScoreXml);
  const cursorPosition = usePracticeStore((s) => s.cursorPosition);
  const listenToRoom = usePracticeStore((s) => s.listenToRoom);
  const setListenToRoom = usePracticeStore((s) => s.setListenToRoom);
  const exploreMeasureLive = usePracticeStore((s) => s.exploreMeasureLive);
  const exploreNotice = usePracticeStore((s) => s.exploreNotice);
  const lessonDrill = usePracticeStore((s) => s.lessonDrill);
  const lessonRecap = usePracticeStore((s) => s.lessonRecap);
  const startLesson = usePracticeStore((s) => s.startLesson);
  const lessonActive = lessonDrill !== null || lessonRecap !== null;
  const exploreActive = usePracticeStore((s) => s.explore !== null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [modeMenuOpen, setModeMenuOpen] = useState(false);
  const [switchError, setSwitchError] = useState<string | null>(null);
  // Catalog is fetched from the backend. Starts as `null` (loading);
  // we don't render the dropdown list items until it resolves.
  const [instruments, setInstruments] = useState<InstrumentInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    invoke<InstrumentInfo[]>("list_instruments")
      .then((list) => {
        if (!cancelled) setInstruments(list);
      })
      .catch(() => {
        // Swallow — the selector screen already surfaces catalog errors;
        // here the worst case is an empty switch menu.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const onPickInstrument = async (name: string) => {
    setMenuOpen(false);
    setSwitchError(null);
    if (!instrumentName || name === instrumentName) return;
    try {
      const inst = instruments.find((i) => i.name === name);
      await switchInstrument(name, inst?.vibratoToleranceCents);
    } catch (err) {
      setSwitchError(String(err));
    }
  };

  /**
   * Changing the mode mid-session closes the current segment and opens
   * a new one under the new mode — so the recorder captures "mode was X
   * for the first Y seconds, then Z". We re-invoke `switchInstrument`
   * with the *current* instrument to trigger that segment boundary.
   */
  const onPickMode = async (mode: PracticeMode) => {
    setModeMenuOpen(false);
    setSwitchError(null);
    if (mode === practiceMode) return;
    setPracticeMode(mode);
    if (!instrumentName) return;
    try {
      const inst = instruments.find((i) => i.name === instrumentName);
      await switchInstrument(instrumentName, inst?.vibratoToleranceCents);
    } catch (err) {
      setSwitchError(String(err));
    }
  };

  const activeMode =
    PRACTICE_MODES.find((m) => m.value === practiceMode) ?? PRACTICE_MODES[1];

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
              {instruments.map((inst) => (
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

        <div className="relative">
          <button
            type="button"
            onClick={() => setModeMenuOpen((v) => !v)}
            data-testid="practice-mode-switch-button"
            aria-expanded={modeMenuOpen}
            aria-haspopup="listbox"
            className="flex items-center gap-1 rounded-full border border-gray-700 bg-gray-800/60 px-3 py-1 text-xs font-medium uppercase tracking-wider text-gray-200 hover:bg-gray-800"
          >
            <span>{activeMode.label}</span>
            <span aria-hidden="true" className="text-gray-500">
              ▾
            </span>
          </button>
          {modeMenuOpen && (
            <ul
              role="listbox"
              data-testid="practice-mode-switch-menu"
              className="absolute left-1/2 top-full z-10 mt-1 w-64 -translate-x-1/2 overflow-hidden rounded border border-gray-700 bg-gray-900 shadow-lg"
            >
              {PRACTICE_MODES.map((mode) => {
                const isActive = mode.value === practiceMode;
                return (
                  <li key={mode.value}>
                    <button
                      type="button"
                      role="option"
                      aria-selected={isActive}
                      onClick={() => void onPickMode(mode.value)}
                      data-testid={`practice-mode-switch-option-${mode.value}`}
                      className={`block w-full px-3 py-2 text-left text-sm hover:bg-gray-800 ${
                        isActive ? "text-blue-300" : "text-gray-200"
                      }`}
                    >
                      <div className="font-medium">{mode.label}</div>
                      <div className="text-xs text-gray-500">
                        {mode.description}
                      </div>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="flex items-center gap-4">
          <SessionTimer />
          {!lessonActive && (
            <button
              type="button"
              onClick={() => void startLesson().catch(console.error)}
              data-testid="start-lesson"
              className="rounded bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500"
            >
              Give me a lesson
            </button>
          )}
          <PocketControl />
          <AccompanimentToggle />
          <EndSessionButton />
        </div>
      </header>

      {/* #392: chord surfaces gate on the instrument's capability. Until the
          catalog resolves (or on an unknown name) fail OPEN — suppressing on
          Piano would be a feature loss; the gate exists to stop mono noise.
          The room exemption holds only while the jam lane is the stage: a
          lesson/explore/score view puts the mic back on the player, and a
          still-set room flag must not keep the noise surfaces alive. */}
      <PerceptionPanel
        instrumentPolyphonic={
          instruments.find((i) => i.name === instrumentName)?.polyphonic ?? true
        }
        roomListening={
          listenToRoom && !lessonActive && !exploreActive && !activeScoreXml
        }
      />

      {lessonActive ? (
        // Guided lesson (#254): the drill's sheet music takes the stage.
        <div className="flex min-h-0 flex-1 flex-col gap-4 lg:flex-row">
          <LessonPanel />
          <div className="flex flex-row items-start gap-6 lg:w-72 lg:flex-col">
            <PitchDisplay />
          </div>
        </div>
      ) : exploreActive ? (
        // Free-play exploration (#255): the variation takes the stage; the
        // pitch meter and reveal stay alongside.
        <div className="flex min-h-0 flex-1 flex-col gap-4 lg:flex-row">
          <ExplorePanel />
          <div className="flex flex-row items-start gap-6 lg:w-72 lg:flex-col">
            <PitchDisplay />
            <div className="flex flex-col gap-4">
              <RevealCard />
            </div>
          </div>
        </div>
      ) : activeScoreXml ? (
        // Score mode: sheet music leads, pitch + tips sit alongside.
        <div className="flex flex-1 flex-col gap-4 p-4 lg:flex-row">
          <div
            className="flex min-h-0 flex-1 flex-col gap-2"
            data-testid="session-score-pane"
          >
            {/* Live hit/near/missed tally (#337 S2) — invisible until the
              follower judges its first note. */}
            <VerdictStrip />
            {/* Latest measure-anchored phrase card (#337 S3, closes #210). */}
            <ScorePhraseCard />
            {/* #341 review M1: a refused measure tap (all rests, too busy)
                must SAY so here — the explore panel that normally shows
                this notice only mounts once an exploration exists. */}
            {exploreNotice && (
              <p
                data-testid="score-explore-notice"
                className="rounded border border-amber-700/60 bg-amber-900/30 px-3 py-1.5 text-xs text-amber-200"
              >
                {exploreNotice}
              </p>
            )}
            <div className="min-h-0 flex-1">
              {/* #341: every measure is a tap target — the in-practice RV
                  bridge. Score-follow sessions only; lessons render their
                  own notation without the overlay. */}
              <ScoreView
                musicXml={activeScoreXml}
                cursorPosition={cursorPosition}
                onMeasureTap={exploreMeasureLive}
              />
            </div>
          </div>
          <div className="flex flex-row items-start gap-6 lg:w-72 lg:flex-col">
            <PitchDisplay />
            <div className="flex flex-col gap-4">
              <CoachingTipPanel />
              <RevealCard />
            </div>
          </div>
        </div>
      ) : (
        // Free play: the pitch meter is the centerpiece, with "work on my
        // last lick" anchored directly beneath it (VA #324 screenshot:
        // parked in the side column it floated in dead space). #329: the
        // side column itself is visible at EVERY width now — on narrow
        // windows it stacks below the centerpiece instead of vanishing
        // (reveals are a flagship; hiding them behind lg: was the same
        // defect class as the button placement #324 fixed).
        // #349 T4a: "Listen to the room" swaps the centerpiece for the jam
        // chord lane — external music is the SIGNAL, and every heard chord
        // is one tap from the RV bridge.
        <div className="flex flex-1 flex-col items-center justify-center gap-8 lg:flex-row lg:gap-12">
          <div className="flex flex-col items-center gap-6">
            {listenToRoom ? <ChordLane /> : <PitchDisplay />}
            {listenToRoom ? null : <LiftLickButton />}
            {/* #419 S1: Openers — the invitation to start with something
                in your hands instead of a blank room. */}
            {listenToRoom ? null : <OpenersPanel />}
            {/* #349 T3c: the progression lift lives beside the lick lift
                (free play) AND under the jam lane — anywhere chords were
                heard, the sequence is one tap from a 12-key row. */}
            <LiftProgressionButton />
            <button
              type="button"
              data-testid="listen-to-room-toggle"
              onClick={() => setListenToRoom(!listenToRoom)}
              className={`rounded-full border px-4 py-1.5 text-xs ${
                listenToRoom
                  ? "border-emerald-500 text-emerald-300"
                  : "border-gray-600 text-gray-400 hover:border-gray-400 hover:text-gray-200"
              }`}
            >
              {listenToRoom
                ? "🎧 Listening to the room"
                : "🎧 Listen to the room"}
            </button>
          </div>
          <div className="flex flex-col gap-4">
            <CoachingTipPanel />
            <RevealCard />
          </div>
        </div>
      )}
    </main>
  );
}
