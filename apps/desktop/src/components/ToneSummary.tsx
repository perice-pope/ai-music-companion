import type { ToneDescriptor } from "../types/brain";

/**
 * Gentle, labelled read-out of the session's tone descriptor.
 *
 * Product invariant ("coach, don't judge", architecture-v2 §4a/§5): tone is
 * surfaced primarily through the coach's *words* in the recap text. This panel
 * is a secondary, calm visual — bars with qualitative labels and **no numeric
 * scores or grades**. Each dimension is a descriptor, not a mark out of ten.
 */
const DIMENSIONS: { key: keyof ToneDescriptor; label: string }[] = [
  { key: "brightness", label: "Brightness" },
  { key: "warmth", label: "Warmth" },
  { key: "core_clarity", label: "Core clarity" },
  { key: "air_noise", label: "Air / breath" },
  { key: "vibrato_quality", label: "Vibrato" },
];

function clamp01(x: number): number {
  return Math.max(0, Math.min(1, x));
}

/** A qualitative word for screen readers — never a number. */
function qualitative(value: number): string {
  if (value < 0.34) return "subtle";
  if (value < 0.67) return "balanced";
  return "pronounced";
}

export default function ToneSummary({ tone }: { tone: ToneDescriptor }) {
  return (
    <div data-testid="recap-tone">
      <h3 className="mb-1 text-sm font-semibold uppercase tracking-wider text-gray-400">
        Tone today
      </h3>
      <p className="mb-3 text-xs text-gray-500">
        A gentle read on how your sound came across — not a grade.
      </p>
      <div className="flex flex-col gap-2">
        {DIMENSIONS.map(({ key, label }) => {
          const value = clamp01(tone[key]);
          return (
            <div key={key} className="flex items-center gap-3">
              <span className="w-28 text-sm text-gray-300">{label}</span>
              <div
                className="h-2 flex-1 overflow-hidden rounded bg-gray-700"
                role="meter"
                aria-label={`${label}: ${qualitative(value)}`}
              >
                <div
                  className="h-full bg-blue-400/70"
                  style={{ width: `${Math.round(value * 100)}%` }}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
