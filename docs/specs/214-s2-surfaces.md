# Spec: Identification feeds the surfaces (#214 S2, E4)

## 1. Summary
A confirmed library match earns real consequences: the reveal card
upgrades from the key-catalog voice to "You're playing — {title}" (the
2a framing line retires ON THAT CARD ONLY while a match is live — the
wrong-Beethoven redemption), and the chip gains "Open score" (stages
the matched score for score practice through the existing selector
flow). Plus the two S1b carried notes: the constructor-tail rebuild pin
and the different-score-while-open echo test.

## 2. Contract
- RevealCard: while `pieceMatch` is live, the header block reads
  "You're playing — {title}" (testid reveal-identified) and the 2a
  framing line ("other music that lives in this sound") is REPLACED by
  "also lives in this key:" above the catalog connection — the catalog
  stays useful, the voice stops hedging. When the match clears or is
  dismissed, the 2a framing returns verbatim (its test keeps passing in
  the no-match state — the S1 promise that the line retires only
  against a real ID, honored literally).
- PieceMatchChip "Open score": invoke get_score for the matched id →
  set activeScore/activeScoreXml → returnToSelector (the staged-score
  flow the picker already speaks). A get_score failure (deleted
  mid-session) shows the chip's calm notice and quiets that id.
- S1b carried notes: (a) with_mocks routes through the same
  constructor tail as build() so the startup rebuild's new()-call is
  pinned; (b) the echo-guard test covers a DIFFERENT score while one
  is open (must surface).

## 3. ACs
1. Live match → reveal header "You're playing — {title}"; framing line
   absent; "also lives in this key:" precedes the catalog connection.
2. Match cleared/dismissed → the 2a framing returns verbatim (the S1
   framing test passes unchanged in that state).
3. Open score: stages the score (activeScore + xml) and lands on the
   selector; failure → calm notice + that id quieted.
4. The startup-rebuild new()-call is pinned (shared constructor tail).
5. Echo guard: a DIFFERENT score than the open one still surfaces.

## 4. Test map
| AC | Test |
|---|---|
| 1 | RevealCard: identified header + retired framing + catalog reframe |
| 2 | RevealCard: the S1 framing test + a clears-and-returns case |
| 3 | PieceMatchChip: open-score happy + failure paths |
| 4 | commands: with_mocks-over-seeded-store identifies WITHOUT a manual rebuild call |
| 5 | PieceMatchChip/store: different-score-while-open surfaces |
