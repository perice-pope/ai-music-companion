/**
 * #349 T4a — the shared confidence→dots mapping (the honesty cue): a
 * hesitant label shows fewer dots, everywhere it appears (live lane and
 * recap chart use the SAME thresholds so they can never disagree).
 */
export function confidenceDots(confidence: number): string {
  if (confidence >= 0.75) return "●●●";
  if (confidence >= 0.55) return "●●";
  return "●";
}
