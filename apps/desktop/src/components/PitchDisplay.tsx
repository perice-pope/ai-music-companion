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

/**
 * #417 rule 0 — surfaces hold their last state; they dim, they never
 * vanish. After a note ends, the meter HOLDS the last reading dimmed for
 * this much AUDIO time (from the event clock — deterministic, no wall
 * clock), then settles to a calm idle that keeps the exact same layout.
 * At a piano every note decays; unmounting on decay strobed the screen.
 */
const HOLD_SECS = 3.0;

/** What the last sounding note looked like, frozen for the held state. */
interface HeldReading {
  name: string;
  octave: number;
  frequencyHz: number;
  cents: number;
  confidence: number;
  /** Audio-clock time of the last voiced event. */
  atSecs: number;
}

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

  // The frozen last reading that carries the held state (#417 rule 0).
  const heldRef = useRef<HeldReading | null>(null);
  if (isListening && currentNote && latestEvent) {
    heldRef.current = {
      name: currentNote.name,
      octave: currentNote.octave,
      frequencyHz: currentNote.frequency_hz,
      cents: smoothedCents ?? currentNote.cents_deviation,
      confidence: latestEvent.confidence,
      atSecs: latestEvent.timestamp_secs,
    };
  }
  if (!isListening) {
    heldRef.current = null; // a new session never inherits a stale needle
  }

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

  const held = heldRef.current;
  // Warm-up: nothing has sounded yet this session — the one moment the
  // skeleton has no reading to hold. Same container, calm copy.
  if (!held) {
    return (
      <div
        className="flex flex-col items-center gap-2"
        data-testid="pitch-display"
      >
        <p className="text-gray-400">Listening...</p>
      </div>
    );
  }

  // Live when a note is sounding; otherwise HELD (dimmed, frozen values)
  // until the hold expires on the audio clock, then IDLE — identical
  // layout throughout: nothing mounts or unmounts with the signal.
  const live = Boolean(currentNote);
  const silentSecs = latestEvent ? latestEvent.timestamp_secs - held.atSecs : 0.0;
  const idle = !live && silentSecs > HOLD_SECS;
  const surface = live ? "live" : idle ? "idle" : "held";

  const { name, octave } = { name: held.name, octave: held.octave };
  const frequency_hz = held.frequencyHz;
  const cents = held.cents;
  const meterOffset = clamp(cents, -50, 50);
  // Convert -50..+50 cents to 0..100% for the meter position
  const meterPercent = ((meterOffset + 50) / 100) * 100;

  return (
    <div
      className={`flex flex-col items-center gap-4 transition-opacity duration-500 ${
        live ? "opacity-100" : idle ? "opacity-30" : "opacity-50"
      }`}
      data-testid="pitch-display"
      data-surface={surface}
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

      {/* Confidence (frozen with the reading while held/idle) */}
      <p className="text-xs text-gray-600">
        confidence: {(held.confidence * 100).toFixed(0)}%
      </p>
    </div>
  );
}
