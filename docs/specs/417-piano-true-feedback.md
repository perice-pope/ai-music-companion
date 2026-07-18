# Spec: Family-aware recap vocabulary (#417 item 4, closes #389)

## 1. Summary
Session recaps speak the instrument's own practice language. Fixed-pitch
instruments (keyboard family) get no player-intonation critique and no
tuner/drone/long-tone/breath advice — pitch deviation is phrased as the
INSTRUMENT's tuning; continuous-pitch instruments are unchanged.

## 2. Problem / why
#389: a 48-min piano session's recap said "Intonation drifted — only 44% of
notes sat in tune" and advised "a tuner drone to settle your pitch" — the
player cannot alter a piano's pitch. #417-4: recap vocabulary generally reads
like a wind instrument. The live TIP path already routes through
instrument-specific prompts (prompts.rs piano branch: voicing, pedal, even
touch); the RECAP path never received the instrument at all.

## 3. Non-goals
- No change to live tips (already correct) or to lesson content.
- No new vocabulary for strings/voice/winds — their banks are appropriate.
- No mallet profiles yet ("Percussion" is pre-wired in `fixed_pitch_family`).

## 4. Contract / interface
- `RecapInput` gains `#[serde(default)] pub instrument_family: String`
  (empty = continuous-pitch = today's behavior; stored inputs unchanged).
- `generate_recap_with_context` gains `instrument_family: String`; the
  command layer resolves it from the instrument catalog
  (`instrument_family_for`, "Piano" → "Keyboard").
- `fixed_pitch_family(family)` = Keyboard | Percussion.
- Offline fallback (`grounded_offline_recap`) and BOTH LLM prompts
  (system: FIXED-PITCH RULES block; user: intonation fact reframed as
  instrument tuning, "NOT player-controllable") route through it.

## 5. Acceptance criteria
1. (#389) A piano (Keyboard) offline recap contains NO player-intonation
   critique and no tuner/drone/long-tone advice — across overall,
   strengths, areas, and suggestions.
2. A strong tuning tendency (|mean| ≥ 10 cents) surfaces phrased as the
   instrument ("your piano reads about N cents flat"), at most as a neutral
   note/tuning-visit suggestion — never as player skill.
3. The same detuned session on trumpet keeps the continuous-pitch bank
   (tuner/drone advice present) — #389's "trumpet unchanged".
4. The key-anchored opener suggestion speaks the family: keyboard = "slow
   scale, hands together"; continuous pitch = "long tones".
5. Empty/unknown family behaves exactly like continuous pitch.
6. LLM prompts: keyboard system prompt carries the fixed-pitch guardrails
   (never critique tuning, no breath vocabulary, speak hands/voicing/pedal);
   the user prompt's intonation fact is reframed for keyboard; both are
   absent for continuous pitch.
7. "Air in the tone" (breath vocabulary) never appears in a keyboard recap.

## 6. Edge cases
- Centered piano (|mean| < 10¢): no tuning mention at all — silence > noise.
- Old stored RecapInputs (no family field): deserialize to "", AC5.
- The in-tune-ratio STRENGTH ("solid intonation") is also gated off for
  fixed pitch — it measures the instrument, praise would be as hollow as
  the critique.

## 7. Test plan
| AC | Test |
|---|---|
| 1,2,7 | coaching `piano_offline_recap_never_critiques_player_intonation` |
| 3 | coaching `trumpet_offline_recap_keeps_the_continuous_pitch_bank` |
| 4 | coaching `opener_suggestion_speaks_the_familys_language` |
| 5 | coaching `unknown_family_defaults_to_continuous_pitch_behavior` |
| 6 | coaching `recap_system_prompt_gates_fixed_pitch_rules_by_family`, `recap_user_prompt_reframes_intonation_for_fixed_pitch` |
| 1,3 e2e | commands `offline_piano_recap_never_suggests_a_tuner` (family from the real catalog) |

## 8. Architecture
Family is a catalog fact owned by the command layer; brain receives it as
data (no registry duplication). All phrase banks stay in Rust. Offline,
no network changes.
