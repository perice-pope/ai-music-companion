/** Small display helpers. Shared rule (spec §4 "Honest absences"): a NULL
 * is rendered as an em dash, never coerced to 0 — no data is not zero. */

export const EM_DASH = "—";

/** The heat window: the last `n` calendar days ending today, as YYYY-MM-DD
 * date keys, oldest first. Matches v_roster_heat's date_key grain. */
export function recentDateKeys(n: number, today: Date = new Date()): string[] {
  const keys: string[] = [];
  for (let i = n - 1; i >= 0; i--) {
    const d = new Date(today);
    d.setDate(d.getDate() - i);
    keys.push(toDateKey(d));
  }
  return keys;
}

export function toDateKey(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** "Mon 7/20" — column header for a date key (parsed as a local date). */
export function dayLabel(dateKey: string): string {
  const [y, m, d] = dateKey.split("-").map(Number);
  const date = new Date(y, m - 1, d);
  const weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][
    date.getDay()
  ];
  return `${weekday} ${m}/${d}`;
}

export function fmtMinutes(min: number | null): string {
  if (min === null) return EM_DASH;
  return min >= 10
    ? String(Math.round(min))
    : String(Math.round(min * 10) / 10);
}

export function fmtSecsAsMin(secs: number | null): string {
  return fmtMinutes(secs === null ? null : secs / 60);
}

export function fmtRatio(ratio: number | null): string {
  if (ratio === null) return EM_DASH;
  return `${Math.round(ratio * 100)}%`;
}

export function fmtCount(n: number | null): string {
  return n === null ? EM_DASH : String(n);
}

export function fmtDateTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** Tonic pitch-class → display name (sharp spelling; display-only). */
export const TONIC_NAMES = [
  "C",
  "C#",
  "D",
  "D#",
  "E",
  "F",
  "F#",
  "G",
  "G#",
  "A",
  "A#",
  "B",
] as const;
