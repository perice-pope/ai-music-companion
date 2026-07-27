import { useEffect, useRef, useState } from "react";
import { useAudioStore } from "../stores/audioStore";
import { useWarmupStore, WARMUP_SECONDS } from "../stores/warmupStore";
import type { WarmupChallengeDto } from "../types/brain";

/** Percent formatting, same voice as the lesson recap's grades. */
function pct(x: number): string {
  return `${Math.round(x * 100)}%`;
}

function clock(secs: number): string {
  const s = Math.max(0, secs);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/**
 * The live throw: label, pacing countdown, and the finish/close controls.
 * Mounted only while the warmup phase is "active", so every throw starts
 * its own fresh clock — a re-throw after an unheard expiry must not
 * inherit the previous throw's expired countdown.
 */
function ActiveThrow({ challenge }: { challenge: WarmupChallengeDto }) {
  const heardCount = useWarmupStore((s) => s.playedNotes.length);
  const notice = useWarmupStore((s) => s.notice);
  const submitting = useWarmupStore((s) => s.submitting);
  const finishWarmup = useWarmupStore((s) => s.finishWarmup);
  const expireUnheard = useWarmupStore((s) => s.expireUnheard);
  const closeWarmup = useWarmupStore((s) => s.closeWarmup);

  const [secondsLeft, setSecondsLeft] = useState(WARMUP_SECONDS);
  useEffect(() => {
    const id = setInterval(
      () =>
        setSecondsLeft((s) => {
          // The clock stops AT zero — ticking past it would re-run the
          // expiry effect every second, auto-retrying a failed completion
          // forever against a backend that just said no.
          if (s <= 1) {
            clearInterval(id);
          }
          return s - 1;
        }),
      1000,
    );
    return () => clearInterval(id);
  }, []);

  // The countdown paces, it doesn't punish: at 0:00 a heard take is graded,
  // an unheard one costs nothing (the throw is free, spec §6). One-shot:
  // expiry fires once per throw — a failed grade waits for the player's own
  // retry tap, and a note played after an error must not re-trigger it.
  const expiredRef = useRef(false);
  useEffect(() => {
    if (secondsLeft <= 0 && !expiredRef.current) {
      expiredRef.current = true;
      if (heardCount > 0) {
        void finishWarmup();
      } else {
        expireUnheard();
      }
    }
  }, [secondsLeft, heardCount, finishWarmup, expireUnheard]);

  return (
    <div
      className="mx-auto flex w-full max-w-xl flex-col items-center gap-4 self-center rounded-lg border border-amber-700/50 bg-gray-800/40 p-6"
      data-testid="daily-warmup-panel"
    >
      <div className="flex w-full items-start justify-between">
        <p className="text-xs font-medium uppercase tracking-wider text-amber-400">
          Daily warmup — today&apos;s throw
        </p>
        <button
          type="button"
          onClick={closeWarmup}
          data-testid="warmup-close"
          aria-label="Close warmup"
          className="text-gray-500 hover:text-gray-300"
        >
          ✕
        </button>
      </div>
      <p
        className="text-2xl font-semibold text-white"
        data-testid="warmup-label"
      >
        {challenge.label}
      </p>
      <p className="text-sm text-gray-400">
        Play it once, up and down — then tap done.
      </p>
      <p
        className="font-mono text-3xl font-bold text-amber-300"
        data-testid="warmup-countdown"
      >
        {clock(secondsLeft)}
      </p>
      <p className="text-xs text-gray-500" data-testid="warmup-heard">
        {heardCount === 0
          ? "Listening…"
          : `Heard ${heardCount} ${heardCount === 1 ? "note" : "notes"}`}
      </p>
      {notice && (
        <p
          className="text-xs text-red-400"
          role="alert"
          data-testid="warmup-notice"
        >
          {notice}
        </p>
      )}
      <button
        type="button"
        onClick={() => void finishWarmup()}
        disabled={submitting}
        data-testid="warmup-finish"
        className="rounded bg-amber-600 px-5 py-2 text-sm font-semibold text-white hover:bg-amber-500 disabled:opacity-50"
      >
        {submitting ? "Grading…" : "I'm done"}
      </button>
    </div>
  );
}

/**
 * #257 S4 — the Daily Warmup Roulette's stage: the thrown key+scale, a
 * ~60 s pacing countdown, and the graded outcome. All grading and streak
 * math is Rust-side; this panel collects the live stream and steps the
 * warmup store's state machine.
 */
export default function DailyWarmupPanel() {
  const phase = useWarmupStore((s) => s.phase);
  const challenge = useWarmupStore((s) => s.challenge);
  const result = useWarmupStore((s) => s.result);
  const hearEvent = useWarmupStore((s) => s.hearEvent);
  const closeWarmup = useWarmupStore((s) => s.closeWarmup);
  const startWarmup = useWarmupStore((s) => s.startWarmup);
  const latestEvent = useAudioStore((s) => s.latestEvent);

  // Feed the live stream into the collector. The store ignores events
  // outside the active phase, so this needs no gating here.
  useEffect(() => {
    if (latestEvent !== null) {
      hearEvent(latestEvent);
    }
  }, [latestEvent, hearEvent]);

  if (phase === "done" && result) {
    return (
      <div
        className="mx-auto w-full max-w-md self-center rounded-lg border border-amber-600/60 bg-amber-950/30 p-6 text-center"
        data-testid="warmup-result"
      >
        <h2 className="text-lg font-semibold text-amber-100">
          Warmup done <span aria-hidden="true">🔥</span>
        </h2>
        <p
          className="mt-3 text-4xl font-bold text-amber-200"
          data-testid="warmup-grade"
        >
          {pct(result.score)}
        </p>
        <p
          className="mt-3 text-sm text-amber-200/80"
          data-testid="warmup-streak-line"
        >
          {result.streak.count === 1
            ? "Day 1 of a new streak — come back tomorrow."
            : `${result.streak.count} days in a row — come back tomorrow.`}
        </p>
        <button
          type="button"
          onClick={closeWarmup}
          data-testid="warmup-done-close"
          className="mt-6 rounded bg-amber-600 px-4 py-2 text-sm font-semibold text-white hover:bg-amber-500"
        >
          Done
        </button>
      </div>
    );
  }

  if (phase === "unheard") {
    return (
      <div
        className="mx-auto w-full max-w-md self-center rounded-lg border border-gray-700 bg-gray-800/60 p-6 text-center"
        data-testid="warmup-unheard"
      >
        <p className="text-sm text-gray-300">
          We didn&apos;t hear you play — no worries, the throw is free.
        </p>
        <div className="mt-5 flex items-center justify-center gap-3">
          <button
            type="button"
            onClick={() => void startWarmup().catch(console.error)}
            data-testid="warmup-try-again"
            className="rounded bg-amber-600 px-4 py-2 text-sm font-semibold text-white hover:bg-amber-500"
          >
            Throw again
          </button>
          <button
            type="button"
            onClick={closeWarmup}
            data-testid="warmup-unheard-close"
            className="rounded border border-gray-600 px-4 py-2 text-sm text-gray-300 hover:border-gray-400"
          >
            Not today
          </button>
        </div>
      </div>
    );
  }

  if (phase !== "active" || challenge === null) {
    return null;
  }

  return <ActiveThrow challenge={challenge} />;
}
