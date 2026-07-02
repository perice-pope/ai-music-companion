# Spec: Reveal card matches the live "I hear" key (#266)

> Bug fix on the shipped Reveal loop (#253 S1), found in VA desktop test #265.

## 1. Summary
A reveal card must never contradict the live "I hear" header. Tie each reveal to the key that
produced it and dismiss it the moment the live detected key/mode moves off it; only fire reveals on
reasonably confident readings.

## 2. Problem / why
Reveal and header read the same `perception.key` but at different times: a reveal is a snapshot that
lingers ~12 s while the header updates ~8 Hz. Early-session key detection wanders (e.g. E → G#
Locrian → D# Phrygian), so a lingering card shows a key the header has already left — the tester saw
"A# Dorian (header) vs F Phrygian (card)". The 0.6 confidence gate fires on unsettled readings.

## 3. Non-goals
- Coaching tips share the same latent staleness — **out of scope** here (tracked separately).
- No change to the cadence, the curated table, or the LLM/S2 path.
- Not solving key-detection wander itself (a perception concern); we just stop showing stale cards.

## 4. Contract / interface
`brain::connections::Reveal` gains the generating key so the frontend can compare it to live
perception:
```rust
pub struct Reveal {
    pub concept: String,
    pub connection: String,
    pub why: String,
    pub source: RevealSource,
    pub tonic: u8,     // NEW: 0-11, the key the reveal was generated for
    pub mode: String,  // NEW: normalized (lowercased) mode, e.g. "dorian"
}
```
TS mirror (`types/brain.ts`) adds `tonic: number; mode: string;`. `REVEAL_MIN_CONFIDENCE` is raised
from `0.6` to `0.72`.

## 5. Acceptance criteria (numbered, testable)
1. `reveal_for` populates `tonic` with `ctx.tonic` and `mode` with the normalized mode it matched
   (e.g. context mode "Dorian" → `mode == "dorian"`, `tonic == 7` for G).
2. Confidence gate: `reveal_for` returns `None` below `0.72` and `Some` at exactly `0.72`.
3. Frontend: when a reveal is showing and `perception.key` becomes a **different** `(tonic, mode)`
   (case-insensitive on mode), the card is dismissed (queue emptied).
4. Frontend: while `perception.key` is **unchanged** or **null** (silence), a showing card is **not**
   dismissed by this logic (it still auto-dismisses on its own linger timer).
5. Preserved: replace-not-stack, ≤1 per N phrases, no card on atonal/low-confidence.

## 6. Edge cases & failure modes
- Live key flips to a different mode but same tonic (or vice-versa) → dismiss (any component differs).
- Live key goes null mid-linger (player pauses) → keep the card (don't punish silence).
- Same key re-detected → no spurious dismiss.
- Mode case differences ("Dorian" vs "dorian") must not count as a change.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `connections::tests::reveal_reports_generating_key` | tonic + normalized mode on the Reveal |
| AC2 | `connections::tests::confidence_at_threshold_reveals` (updated to 0.72) + `low_confidence_returns_none` | new gate |
| AC3 | `RevealCard.test.tsx` "dismisses when the live key moves off it" | queue emptied on key change |
| AC4 | `RevealCard.test.tsx` "keeps the card when key is unchanged or silent" | not dismissed |
| AC5 | existing supersede / cadence / atonal tests still green | no regression |

## 8. Architecture / approach
Selection/gating stays in Rust core (`reveal_for`). The card **lifecycle** (dismiss-on-key-change) is
presentation and lives in `RevealCard.tsx`, which already subscribes to the store — it reads
`perception.key` and calls `dismissReveal` when it no longer matches the shown reveal. No new IPC, no
network. Offline unaffected.

## 9. Slice breakdown
Single slice (small): backend field + threshold, TS mirror, card lifecycle, tests.

## 10. Risks / open questions
- Raising the gate to 0.72 reduces reveal frequency; acceptable — accuracy > volume. Revisit if too
  quiet.

## 11. References
- #253 (Reveal S1), #265 (VA test), `crates/brain/src/connections.rs`, `RevealCard.tsx`,
  `PerceptionPanel.tsx` (the live header this must agree with).
