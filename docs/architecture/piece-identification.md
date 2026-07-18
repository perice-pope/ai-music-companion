# Piece Identification — "know the piece, listen accordingly" (#417 item 5, #214, #208)

**Status:** design doc, pre-implementation. First slice scoped at the end.
**Author's stance:** this is the strategic feature of the listening stack. Everything
the founder flagged in #417 — the wrong-Beethoven reveal, generic feedback, the
lesson picker's blindness — collapses into one capability: *the app knows what
you're playing.*

---

## 1. The founder's ask, and the honest technical answer

> "There is already technology out there like Shazam… if the app knows you're
> playing Für Elise, everything should follow."

The ask is right; the named technology is not the path. **Shazam-class audio
fingerprinting matches recordings, not performances.** It hashes the spectral
fingerprint of one specific master recording; your Für Elise, at your tempo, on
your piano, in your room, shares no fingerprint with any released recording.
AcoustID/Chromaprint have the same contract. That entire industry solves
"which recording is this?" — we need "which *piece* is this?"

What does transfer is what we already build: **score alignment.** Our follower
(`crates/brain/src/follower.rs`) is Online DTW aligning live `AudioEvent`s to a
`ScoreModel` with ±20% tempo tolerance, <3 ms per step. Today it follows ONE
loaded score. Piece identification is the same machine pointed at a *set* of
scores: retrieval first (cheap features narrow thousands of candidates to a
few), then confirmation (the real follower aligns the finalists and the best
alignment wins). Identification is not a new subsystem — it is **retrieval in
front of the follower we already trust.**

## 2. Product shape: identification feeds every surface

When identification asserts (and only then — §5):

- **The reveal card (#417 item 2b).** Today's card is a key catalog wearing a
  disclaimer ("other music that lives in this sound", shipped in 2a). With an
  ID, the card earns the direct voice: *"You're playing — Für Elise, Beethoven"*
  with a "pull up the score" action. The 2a framing line is deliberately
  load-bearing until this ships; this is the feature that retires it.
- **The score appears (#214's UX).** One tap opens the matched score with the
  follower already aligned at your current measure — mid-piece, not page one.
- **Piece-aware feedback.** With an alignment, critique gets an address:
  "the left hand rushed the arpeggios in measure 12" instead of "timing
  wandered". This composes with #417-4's family vocabulary: family decides the
  *language*, alignment supplies the *where*.
- **The lesson picker.** "Drill the B section you struggled with" — lessons
  seeded from the identified piece's own material, which is the RV loop closing:
  the piece IS the cell source.

## 3. Architecture

### 3.1 Features: transposition- and tempo-invariant melodic n-grams

Index unit: **interval n-grams** over the melodic surface of each score part.

- From each `ScoreModel`, extract the note sequence per part; for polyphonic
  scores take the top line per onset (melody approximation) — chord tones
  collapse to their highest note, matching what a listener tracks.
- Emit sliding windows of `N=5` consecutive **intervals** (semitone deltas,
  clamped to ±12): Für Elise's opening E–D#–E–D#–E–B–D–C–A becomes
  `[-1,+1,-1,+1]`, `[+1,-1,+1,-4]`, … Intervals, not pitches → the player
  practicing in another key still matches (RV: the cell rowed through 12 keys
  must not defeat identification of the cell's source). No durations in v1 →
  tempo-free by construction; rhythm becomes a tiebreaker later, not a gate.
- Each n-gram hashes to a 64-bit key → posting list of `(score_id, position)`.
  A 200-piece library is a few hundred KB; the shipped-corpus future (~500
  works) stays comfortably in memory. Index lives in the score store (SQLite),
  built at import time and backfilled by a one-time migration for existing
  libraries.

### 3.2 Query: the live stream, same pipeline

The session already produces a monophonic-first note stream (`AudioEvent`s) and,
on polyphonic instruments, the T3 chord/bass path. Query features mirror the
index: top-line note onsets → interval n-grams over a sliding window of the
last ~20 notes. Every few seconds (not per-onset — identification is ambient,
not real-time-critical), look up the window's n-grams, score candidates by
weighted hit count with positional coherence (hits that agree on "where in the
piece" count double — this is what separates *playing Für Elise* from *playing
a scale Für Elise also contains*).

### 3.3 Confirmation: the follower is the judge

Retrieval's top 2–3 candidates each get a real Online DTW alignment over the
recent note history (the follower run offline over a buffer, not live). Accept
iff the best candidate's alignment cost clears an absolute bar AND leads the
runner-up by a margin. This two-stage shape is why the design is cheap: the
expensive, trusted judge only ever sees a handful of candidates.

### 3.4 What identification is NOT built on

- Not chroma fingerprints of audio (recording-matching, see §1).
- Not the LLM (identification is a measurement; the LLM may *narrate* a
  confirmed ID, never produce one — the reveal grounding rule already says this).
- Not online lookup. Fully offline, like everything else (index is local;
  the future starter corpus ships in the bundle or as an opt-in download with
  full disclosure per `offline-first-and-network-transparency.md`).

## 4. Slices

| # | Slice | Contents | Payoff |
|---|---|---|---|
| **S1** | **Library match** | n-gram index in score store + import-time indexing + retrieval + follower confirmation + one surface: a calm session chip "sounds like *Title* from your library — open it?" | Proves the UX on the user's own scores. Zero licensing, zero new infra, entirely our code. |
| S2 | Reveal integration (2b) | Confirmed ID replaces the key-catalog card; 2a framing line retires ON THIS CARD ONLY; "pull up the score" opens aligned | The wrong-Beethoven complaint dies for library pieces |
| S3 | Piece-aware feedback | Alignment feeds recap/tips with measure addresses | "measure 12" critique |
| S4 | Starter corpus | ~500 public-domain piano works (MusicXML), shipped/opt-in; same index | Works out of the box for the classical canon |

S1 acceptance sketch (the spec will formalize):
1. Importing a score builds its n-gram index; deleting removes it.
2. Playing ≥ ~12 notes of a library piece (any key, reasonable tempo) surfaces
   the chip with the right title; the follower-confirmation margin is pinned.
3. Playing free material (scales, noodling, an unknown piece) surfaces nothing —
   the false-positive test is a first-class AC, not an afterthought.
4. The chip obeys #417 rule 0: it appears, holds, dims — never flashes.
5. Fully offline; no new network calls.

## 5. Honesty rules (non-negotiable, from the RV philosophy)

- **Silence beats a wrong ID.** The wrong-Beethoven card was worse than
  nothing; a wrong "You're playing X" would be worse still. Assert only above
  the confirmation margin; below it, say nothing (the 2a catalog card remains
  the fallback voice).
- **An ID, once asserted, is sticky** — it dims on contradiction and is
  replaced by a better ID; it never blinks out (#417 rule 0).
- **Never fake precision.** "sounds like" phrasing until the follower has
  confirmed alignment; "You're playing" only after.

## 6. Open questions (flagged for the S1 spec)

1. Melody extraction on heavy-pedal piano input: the top-line approximation may
   need the T3 chord path as a fallback query feature (chord-root n-grams).
   S1 ships melody-first and measures.
2. Window/threshold tuning needs real recordings — the VA's piano fixtures
   pipeline (#382) is the template: her recordings become the calibration set.
3. Index versioning: store an index schema version; rebuild lazily on bump.
4. Where the chip lives (session strip vs reveal stack) — founder call.
