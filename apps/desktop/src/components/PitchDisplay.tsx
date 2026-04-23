import { useAudioStore } from "../stores/audioStore";

/** Clamp a value between min and max. */
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

// NOTE: The note name and cents-deviation text are intentionally rendered in a
// neutral color. Per-note green/yellow/red is the Tonestro/SmartMusic pattern
// we explicitly do NOT do — see `docs/architecture/architecture-v2.md` §4a and
// §8 ("We do not color individual notes green/yellow/red during performance...
// That's the Tonestro/SmartMusic approach and users hate it. We assess phrases,
// not notes."). Coaching judgments happen at the phrase layer, surfaced via
// CoachingTipPanel / SessionRecap.
//
// The small continuous tuning-meter indicator below keeps its color: that's a
// real-time tuning aid (needle position), not a per-note judgment.

export default function PitchDisplay() {
  const { currentNote, latestEvent, isListening } = useAudioStore();

  if (!isListening) {
    return (
      <div className="flex flex-col items-center gap-2" data-testid="pitch-display">
        <p className="text-gray-500">Not listening</p>
      </div>
    );
  }

  if (!currentNote || !latestEvent) {
    return (
      <div className="flex flex-col items-center gap-2" data-testid="pitch-display">
        <p className="text-gray-400">Listening...</p>
      </div>
    );
  }

  const { name, octave, cents_deviation, frequency_hz } = currentNote;
  const meterOffset = clamp(cents_deviation, -50, 50);
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
      {/* Note name — neutral color; coaching judgments happen at the phrase layer */}
      <div className="text-center">
        <span
          className="text-6xl font-bold text-gray-100"
          data-testid="pitch-note-name"
        >
          {name}
        </span>
        <span className="text-2xl text-gray-400">{octave}</span>
      </div>

      {/* Frequency */}
      <p className="text-sm text-gray-500">{frequency_hz} Hz</p>

      {/* Cents deviation — neutral numeric readout, not a verdict */}
      <p
        className="text-lg font-mono text-gray-300"
        data-testid="pitch-cents-text"
      >
        {cents_deviation > 0 ? "+" : ""}
        {cents_deviation.toFixed(1)} cents
      </p>

      {/* Pitch meter bar */}
      <div className="relative h-3 w-64 rounded-full bg-gray-700">
        {/* Center line */}
        <div className="absolute left-1/2 top-0 h-full w-0.5 -translate-x-1/2 bg-gray-500" />
        {/* Indicator */}
        <div
          className={`absolute top-0 h-full w-2 -translate-x-1/2 rounded-full ${
            Math.abs(cents_deviation) <= 10
              ? "bg-green-400"
              : Math.abs(cents_deviation) <= 25
                ? "bg-yellow-400"
                : "bg-red-400"
          }`}
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
