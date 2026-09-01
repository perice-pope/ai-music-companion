# Spec: mid-session instrument switch moves the voiced gate with the detector (#521)

## 1. Summary
A mid-session instrument switch hot-swaps the pitch detector but silently keeps the
previous instrument's voiced-confidence gate on the phrase aggregator. Move the gate
with the rest of the profile so the #185 fix holds across `switch_instrument`.

## 2. Problem / why
`DetectorProfile.voiced_confidence_threshold` is carried on the reconfigure channel and
documented as "fed to the phrase aggregator", but the worker loop's reconfigure branch
(`apps/desktop/src-tauri/src/audio_pipeline.rs`, `drain_latest(&profile_rx)`) applies only
`into_pitch_config` — the gate is dropped. Trumpet (0.5) → Voice (0.3) keeps the 0.5 gate:
breathy singing in the 0.3–0.5 band forms zero phrases, so the Voice segment records
nothing ("you didn't play", #185's exact failure). Voice → Trumpet keeps the loose 0.3
gate instead. Found by #509's adversarial review (finding 2); filed as #521.

## 3. Non-goals
- Profile-load validation of `voiced_confidence_threshold` (#509 finding 1 — awaiting a
  product call).
- Re-deriving any other aggregator state on switch (key tracker, note gate, open phrase):
  the session is continuous; only the gate is per-instrument profile data.
- Frontend changes. The switch flow already exists end to end.

## 4. Contract / interface
- New: `PhraseAggregator::set_voiced_confidence_threshold(&mut self, f64) -> Result<(), PhraseError>`
  (crates/brain). Validates via the existing `PhraseConfig::validate` rules; on `Err` the
  previous gate stays in force. Applies to subsequent `push`es only — no retroactive
  re-judging of buffered events.
- Changed behavior (no signature change): the worker loop's reconfigure branch applies the
  incoming profile's gate to the aggregator when (and only when) the detector rebuild
  succeeds — one profile, applied whole or not at all, modulo AC3.

## 5. Acceptance criteria (numbered, testable)
1. After a reconfigure to a looser gate, events whose confidence clears the NEW gate (but
   not the old one) count as voiced and form phrases.
2. After a reconfigure to a stricter gate, events below the new gate no longer count as
   voiced (a session that formed phrases before the switch forms none after).
3. An invalid gate arriving via reconfigure is rejected with a warning; the previous gate
   stays in force and the worker keeps running.
4. A rejected detector rebuild leaves the whole previous profile in force — gate included.
5. Setter unit contract: invalid values (0, negative, >1, NaN) return
   `PhraseError::InvalidConfidenceThreshold` carrying the offending value, and the gate is
   observably unchanged afterward.

## 6. Edge cases & failure modes
- Gate change while a phrase is open: the new gate judges subsequent events; the open
  phrase closes by the normal silence/measure rules. No re-judging of past events.
- Reconfigure with both a bad pitch range and a bad gate: detector rejected → nothing
  applied (AC4 subsumes it).
- No pipeline running (mic failed at start): unchanged — `reconfigure_audio_pipeline`
  is already a no-op there.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `audio_pipeline::tests::reconfigure_moves_the_voiced_gate_with_the_detector` | strict-gate session emits 0 phrases until the mid-loop reconfigure to a loose gate, then phrases form |
| AC2 | `brain::phrase` unit `tightened_gate_stops_counting_borderline_events_as_voiced` | 0.4-confidence events form phrases at gate 0.3, none after tightening to 0.5 |
| AC3 | `audio_pipeline::tests::invalid_reconfigured_gate_keeps_the_previous_gate` | NaN gate in an otherwise-valid reconfigure: worker survives, phrases still form under the initial gate |
| AC4 | `audio_pipeline::tests::rejected_detector_rebuild_keeps_the_previous_gate` | reconfigure with invalid pitch range + loose gate: strict initial gate stays (0 phrases) |
| AC5 | `brain::phrase` unit `set_voiced_gate_rejects_invalid_values_and_keeps_the_old_gate` | typed error carries the value; behavior under the old gate unchanged |
| AC1 (unit) | `brain::phrase` unit `loosened_gate_counts_previously_subvoiced_events` | same event stream flips unvoiced→voiced across the setter call |

The pipeline tests drive the real `worker_loop` with scripted sine PCM and send the
reconfigure from inside the emit callback (same thread), so the gate change lands
mid-session through the production seam. Gate values are derived from the detector's
measured confidence on the same PCM, so the tests can't silently pass on a confidence
drift.

## 8. Architecture / approach
Pure local Rust; no network, no allocation changes in the hot path (the setter is called
on the processing thread only, from the existing reconfigure branch). Matches the
detector-rejection contract: bad data warns and keeps the previous state, never kills the
worker (#509).

## 9. Slice breakdown
One slice — this PR.

## 10. Risks / open questions
None open. #509 finding 1 (validate at profile load) stays with the founder.

## 11. References
#521, #509 (finding 2), #185, `crates/brain/src/phrase.rs`,
`apps/desktop/src-tauri/src/audio_pipeline.rs`, `profiles/voice.json`.
