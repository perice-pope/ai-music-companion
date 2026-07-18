# Spec: Openers live-key tonic + pattern directions (#419 S2b)

## 1. Summary
The opener rows from the key you're actually in (when confidently heard;
C otherwise), and the Pattern-directions bank entry goes live as a
recipe-level modifier: forward / reversed / varied (random per root).

## 2. Problem / why
S1/S2a row from C always ("openers speak in abstract degrees"). A player
noodling in A who builds an opener should start the row in A — RV's own
behavior. Direction is the last resting bank entry short of My Patterns.

## 3. Non-goals
- No enclosure-as-modifier (enclosure is an ITEM since S2a).
- No per-item directions — direction applies to the whole composite,
  matching variations::generate semantics.
- No My Patterns (S3), no persistence (S4).

## 4. Contract / interface
- `start_explore_cell` gains `direction: DirectionMode`; existing
  callers pass Forward (behavior unchanged, compiler-enforced).
- `preview_opener`/`begin_opener` gain `tonic: Option<u8>` (pitch class
  0-11) and `direction: Option<String>` ("forward"|"reversed"|"varied");
  unknown direction strings refuse calmly; missing = Forward; missing/
  invalid tonic = 0 (C). tonic is folded % 12 defensively.
- Store: `_refreshOpenerPreview` reads the live key ONCE per refresh —
  `perception.key` with confidence >= 0.55 (the same assert threshold
  the reveal card uses) — captures it as `openerTonic`, and `beginOpener`
  sends the CAPTURED tonic, not a fresh read: the preview IS the
  exercise, even if the room's key drifts between preview and Begin.
- Panel: a "Pattern direction" row of three exclusive chips (forward
  default, reversed, varied); the selected direction is sent on both
  preview and begin; COMING_SOON shrinks to My patterns only.

## 5. Acceptance criteria
1. begin/preview with tonic Some(9) rows from A (dto tonic/labels);
   None or low-confidence key → C exactly as today.
2. The tonic used by Begin equals the one the LAST preview used
   (captured, not re-read) — pinned against a moved perception key.
3. direction "reversed" reverses the figure per root; "varied" shuffles
   direction per root deterministically under the seed; "forward"
   byte-identical to today.
4. Preview and Begin with the same items+tonic+direction produce the
   same dto (determinism holds with the new params).
5. Non-opener explore paths (lift, measure) are byte-identical
   (Forward passed, pinned by existing suites).
6. Panel: direction chips are exclusive, default forward, wire the
   chosen value on preview and begin; chip state resets after Begin.
7. Unknown direction string over the wire → calm named refusal.

## 6. Edge cases
- tonic 128 over the wire → % 12 fold (defensive; the store never sends
  it, but the wire shouldn't panic).
- Key confidence exactly at the threshold → included (>=).
- Direction chosen, then all items removed ONE BY ONE → preview clears;
  direction chip stays (it's a setting, not an item). The store-level
  clearOpener() API resets the WHOLE builder including direction (review
  round-3 note: if a Clear button ever wires to it, that is the
  deliberate semantic — reset, not clear-items).

## 7. Test plan
| AC | Test |
|---|---|
| 1 | commands: opener_impl tonic Some(9) → explore.tonic 9; None → 0 |
| 2 | store test: perception key A at preview, moved to D before begin → begin sends 9 |
| 3 | commands/brain: reversed cell figure reversed; varied differs from forward but is seed-stable |
| 4 | commands: preview==begin dto with same params (extend S1 purity test) |
| 5 | existing lift/measure suites unchanged |
| 6 | panel: chip exclusivity + wire shape on preview/begin |
| 7 | commands: direction "sideways" → calm refusal |

## 8. Architecture
Tonic is measured by the backend but displayed/held by the frontend —
the frontend passes the semantic VALUE it already renders (same pattern
as items); all interpretation (fold, default, direction mapping) stays
in Rust. No new state in AppState.
