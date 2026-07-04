# The Random Variations method — what this app is actually teaching

> Founder guidance (2026-07-04; he and RV's author both hold masters in music theory). This is the
> product's north star: read it before designing any practice feature.

## The unit of practice is the cell, not the key

RV is **12-tone playing**. Every exercise starts from a **12-tone row** — all twelve keys, shuffled
so muscle memory can't carry the player — and a **cell**: a small piece of musical material.

A cell can be:
- a scale fragment, triad, or arpeggio (the built-in catalogs in `crates/variations`),
- an interval pattern or enclosure,
- **a phrase the player just played** — the most powerful case.

The method is: **cell × row × modifiers**. Take the cell, transpose it through the shuffled 12-key
row, then layer enclosures / intervals / direction randomization on top. Difficulty is how much of
the row you face, how fast, and how many modifiers stack — not "harder keys".

## The flagship loop (phrase-seeded variations)

> "When we hear the user play a phrase we think they should work on — we take that, transpose it to
> 12 keys, and that's their starter exercise. Then add enclosures or intervals on top. Like roulette."

Hear → lift the phrase as a cell → row it through 12 keys → drill it → stack modifiers. The player
practices *their own music* in every key. `variations::VariationSpec.cell` is the primitive that
enables this: any note sequence, normalized to semitone offsets from its first note, becomes a
figure the generator rows like any catalog material.

## What key detection is (and is not) for

Playing a phrase that momentarily fits C Dorian does **not** mean the music "is in C Dorian" — the
cell is the meaning; the key is where it happens to sit. Tonal-center tracking exists for **display
honesty only**: the "I hear" header, the reveal cards, and the recap must never visibly contradict
each other or flap distractingly. That bar is "calm and consistent", not "harmonically definitive".
Never re-center the practice engine on key detection, and never let improving it block cell work.
