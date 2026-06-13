import { useRef } from "react";
import { useAudioStore } from "../stores/audioStore";

/** Clamp a value between min and max. */
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * EMA factor for the *displayed* cents. Low enough to calm the per-event
 * flicker that made the read-out impossible to read (#187), high enough to stay
 * responsive. This is display-layer smoothing only — the real-time pitch
 * detection path is untouched.
 */
const CENTS_SMOOTHING_ALPHA = 0.25;

export default function PitchDisplay() {
  const { currentNote, latestEvent, isListening } = useAudioStore();
  const rawCents = currentNote?.cents_deviation ?? null;

  // Exponential moving average of the displayed cents so the number glides
  // instead of snapping on every audio event (#187). Held in a ref so each
  // audio event folds into the existing value during render — no extra
  // re-render, no one-frame lag behind the store. The first reading of a note
  // shows exactly (no startup lag); subsequent readings are damped. Resets to
  // null when idle (not listening) or when the note drops out so a new session
  // never starts from a stale value. Computed before the early returns below to
  // keep hook order stable.
  const smoothedRef = useRef<number | null>(null);
  if (!isListening || rawCents == null) {
    smoothedRef.current = null;
  } else {
    smoothedRef.current =
      smoothedRef.current == null
        ? rawCents
        : smoothedRef.current +
          CENTS_SMOOTHING_ALPHA * (rawCents - smoothedRef.current);
  }
  const smoothedCents = smoothedRef.current;

  if (!isListening) {
    return (
      <div
        className="flex flex-col items-center gap-2"
        data-testid="pitch-display"
      >
        <p className="text-gray-500">Not listening</p>
      </div>
    );
  }

  if (!currentNote || !latestEvent) {
    return (
      <div
        className="flex flex-col items-center gap-2"
        data-testid="pitch-display"
      >
        <p className="text-gray-400">Listening...</p>
      </div>
    );
  }

  const { name, octave, frequency_hz } = currentNote;
  // Show the smoothed value; fall back to the raw reading defensively (the ref
  // is always seeded above whenever we reach this point).
  const cents = smoothedCents ?? currentNote.cents_deviation;
  const meterOffset = clamp(cents, -50, 50);
  // Convert -50..+50 cents to 0..100% for the meter position
  const meterPercent = ((meterOffset + 50) / 100) * 100;

  return (
    <div
      className="flex flex-col items-center gap-4"
      data-testid="pitch-display"
      role="status"
      aria-live="polite"
      aria-atomic="true"
    >
      {/* Note name */}
      <div className="text-center">
        <span className="text-6xl font-bold text-gray-100">{name}</span>
        <span className="text-2xl text-gray-400">{octave}</span>
      </div>

      {/* Frequency */}
      <p className="text-sm text-gray-500">{frequency_hz} Hz</p>

      {/* Cents deviation (smoothed for readability) */}
      <p className="text-lg font-mono text-gray-300">
        {cents > 0 ? "+" : ""}
        {cents.toFixed(1)} cents
      </p>

      {/* Pitch meter bar */}
      <div className="relative h-3 w-64 rounded-full bg-gray-700">
        {/* Center line */}
        <div className="absolute left-1/2 top-0 h-full w-0.5 -translate-x-1/2 bg-gray-500" />
        {/* Indicator — neutral color, no judgment on deviation */}
        <div
          className="absolute top-0 h-full w-2 -translate-x-1/2 rounded-full bg-gray-400"
          style={{ left: `${meterPercent}%` }}
          data-testid="pitch-meter-indicator"
        />
      </div>

      {/* Labels */}
      <div className="flex w-64 justify-between text-xs text-gray-600">
        <span>flat</span>
        <span>sharp</span>
      </div>

      {/* Confidence */}
      <p className="text-xs text-gray-600">
        confidence: {(latestEvent.confidence * 100).toFixed(0)}%
      </p>
    </div>
  );
}
