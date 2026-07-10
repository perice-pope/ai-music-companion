import { usePracticeStore } from "../stores/practiceStore";

/**
 * #337 S3 (closes #210) — per-phrase feedback in score sessions: the latest
 * phrase's measure-anchored card, text built entirely in Rust ("Measures
 * 5-8 — 6 clean, 1 rough, 2 missed"). Renders nothing until a phrase with a
 * card closes, and never in free play (no phrase ever carries one there).
 */
export default function ScorePhraseCard() {
  const phrases = usePracticeStore((s) => s.phrases);
  const latest = [...phrases].reverse().find((p) => p.score_card);
  if (!latest?.score_card) return null;

  return (
    <div
      className="rounded-md border border-gray-700 bg-gray-900/70 px-3 py-2 text-sm text-gray-200"
      data-testid="score-phrase-card"
    >
      {latest.score_card}
    </div>
  );
}
