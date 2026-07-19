# Spec: the five stable explore chips (#445 pts 4+3, F2)

## 1. Summary
The explore chips morph under the player's finger: slot 3 alternates
between "Different scale" and "Try a pattern" BY SEED PARITY (the seed
advances every rep, so the button flips identity on every tap), the
difficulty slot swaps between "Make it spicy"/"Simpler", chips vanish
when gated, and "Make it spicy" is named nothing like what it does
(BumpDifficulty +1 — which the founder correctly observed "appends
keys"). Founder: five buttons, each doing exactly one named thing,
never trading places. Rule 0 applies to CONTROLS: a chip is a surface —
it holds its identity and DISABLES in place when it can't act.

## 2. The works-once bug (#445-3), root-caused
`generate()` shuffles roots only when `randomize_roots && roots.len()
> 2` — the RV row pins the FIRST root (start where you are), so with
two roots a "shuffle" is a hard no-op. At low difficulty (1–2 roots)
"New keys" does nothing after the first tap's seed advance. The old
row also re-dealt/hid chips per rep, so repeat taps could land on a
different button entirely. Both die here.

## 3. Contract
- `ChipSpec` gains `enabled: bool`. `suggest_chips(state)` (model param
  dropped — no more struggle-based either/or) returns the SAME five, in
  the SAME order, every rep:
  1. **Shuffle 🎲** (ReshuffleRoots) — enabled iff roots > 2 (the
     generator's own no-op boundary, surfaced honestly).
  2. **Add keys** (BumpDifficulty +1) — enabled below MAX_DIFFICULTY.
  3. **Simpler** (BumpDifficulty −1) — enabled above 0.
  4. **Try a pattern 🎲** (TryPattern) — always.
  5. **Different scale** (DifferentScale) — always.
  "Reverse it" leaves the row (it was a <3-chip filler; direction lives
  in the openers + editing surfaces).
- Frontend renders all five always; disabled chips dim (opacity), stay
  in place, and do not fire. Testids disambiguate the two difficulty
  chips (`chip-bump_difficulty-up/-down`).
- Deltas and their semantics are UNCHANGED — this is an identity/
  honesty slice, not a behavior change to any delta.

## 4. ACs
1. suggest_chips returns exactly these five labels in this order for
   ANY (seed, difficulty, roots) — pinned across seed parities (the
   flip mutant dies).
2. Gates as disabled flags: difficulty 0 → Simpler disabled; MAX →
   Add keys disabled; roots ≤ 2 → Shuffle disabled; roots > 2 →
   enabled. Everything else always enabled.
3. Panel: five buttons render even when gated; a disabled chip does
   not invoke; an enabled one applies its exact delta.
4. Existing delta behaviors (reshuffle actually reshuffles at >2
   roots, pattern pulls from the database, scale never repeats) keep
   their pins.

## 5. Test map
| AC | Test |
|---|---|
| 1 | coach: five stable labels at seed 2 AND 3, difficulty 0 AND MAX |
| 2 | coach: enabled matrix (0/MAX/1-root/2-root/3-root) |
| 3 | ExplorePanel: disabled chip no-op + enabled chip fires delta |
| 4 | existing coach chip tests updated to the new shape, kept green |
