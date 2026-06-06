# Story — Phase 4: Musical Understanding & the Cultural Relevance Engine

**Status:** Draft — design proposal for founder review
**Author:** Design proposal generated for review
**Phase:** 4 (follows the Phase 3 tone / teacher / sync work)
**Related:** `docs/architecture/architecture-v2.md`, `story-phase3-tone-quality.md`, `crates/ears`, `crates/brain`, `crates/tone`

---

## 0. North star

> A great teacher doesn't just say "you played a dotted-eighth-sixteenth." They say *"that's the same pocket as a Dilla beat,"* or *"that modal colour is what Miles is doing on Kind of Blue — and also all over film scores you love."* They take what's under your fingers **right now** and connect it to **your** world — the rappers, the movies, the records you already care about — so practice never feels like it's all Bach and Beethoven.

Phase 4 builds that translator. Two halves, in strict order:

1. **Musical Understanding (the ears/brain get smarter)** — real intonation, real rhythm/groove, key & mode. The honest measurement layer.
2. **The Cultural Relevance Engine (the hook)** — an internet-connected LLM that takes the measured musical fingerprint + the student's taste and connects what they just played to the whole recorded history of music, personalised to them.

The order is non-negotiable, and here's why.

## 1. The one principle that makes or breaks this

**Every cultural connection must be grounded in a feature the DSP actually measured.**

The dream feature — "you're playing D dorian, just like *So What*" — is *worse than nothing* if we're wrong about the D dorian. Our most valuable early users are trained musicians (the founder has perfect pitch); they will catch a hallucinated key, a wrong groove, or a made-up "fact" about an artist in **one second**, and never trust the app again. An LLM riffing on vibes will eventually fabricate. So:

- The LLM may only assert musical facts about the performance that come from the **feature bundle** we hand it (key, mode, tempo, groove, intonation, tone, harmonic hints) — each tagged with a **confidence**.
- Low-confidence features are passed as "unsure," and the engine is instructed to **hedge or stay silent**, never invent.
- The LLM's freedom is in the **cultural mapping** (which artists/records/genres relate to these features) and the **encouraging framing** — *not* in deciding what the student played.

This is why Track A (measurement) ships before Track B (relevance): the connections are only as credible as the features under them. Build the BS-proof substrate first.

---

## 2. Track A — Musical Understanding (the measurement layer)

These also stand on their own as the biggest gaps from the current engine (today: monophonic YIN pitch, `stability` used as a proxy for intonation, and `rhythmic_stability` hardcoded to `1.0`). New crate suggestion: **`crates/theory`** (pure-Rust DSP/analysis, no allocation in the hot path where it touches it).

### A1 — Real intonation (not just stability)
- Cents deviation of each sustained note vs **equal temperament**, and, once key is known, vs **just intonation** relative to the tonic (the thing string players and singers actually chase).
- A per-session **tuning-tendency map**: "major thirds +11¢, leading tone consistently flat." This is something even a perfect-pitch teacher can't log objectively for 40 minutes — a genuine machine advantage.
- Replaces the misleading `intonation_tendency = stability` in `crates/brain/src/scoring.rs`.

### A2 — Real rhythm & groove (the part you flagged as essential)
- **Onset timing** against a tracked beat grid (kills the `1.0` placeholder at `scoring.rs:67`). Note: today's onset detector only fires on silence→sound (`ears/src/pitch.rs`), so it misses re-articulation and legato — A2 needs a proper onset/IOI detector.
- **Tempo tracking** + **swing ratio** (straight vs triplet-swung eighths) + **micro-timing / "feel"**: are you laying *back* (behind the beat, like a lot of hip-hop and gospel) or pushing *ahead* (a lot of punk/bluegrass)? This "pocket" measurement is the raw material for every groove-based cultural reference.
- Output: a compact **GrooveDescriptor** (tempo, swing %, mean timing offset ± ms, timing consistency) — descriptors, not a grade, consistent with architecture-v2 §5.

### A3 — Key & mode detection
- Krumhansl–Schmuckler key-profile correlation over the pitch-class histogram → tonic + major/minor, extended to the **modes** (dorian, mixolydian, phrygian…) that define genre colour. Works in free play with **no score loaded** — today key is only known if you import a score.
- Cheap, well-understood, high-leverage: unlocks key-aware intonation (A1) *and* is the single most useful feature for cultural mapping ("mixolydian = blues/rock/Celtic vocabulary").

### A4 — Harmonic hints (stretch / depends on polyphony)
- Chord/interval context for piano and for double stops. Honest dependency: this needs the **polyphonic** path (offline `crates/transcribe` today; real-time multi-f0 is a separate large effort). Scope A4 as "best-effort from whatever polyphony we have," not a promise.

---

## 3. Track B — The Cultural Relevance Engine (the hook)

### 3.1 The musical fingerprint
A compact, **confidence-tagged** struct assembled from Track A + existing tone analysis — the *only* performance facts the LLM is allowed to assert:

```
MusicalFingerprint {
  key: Option<(PitchClass, Mode, confidence)>,
  tempo_bpm: Option<(f64, confidence)>,
  groove: Option<GrooveDescriptor>,        // swing %, lay-back ms, feel
  intonation: IntonationSummary,            // tendencies, in cents
  tone: ToneDescriptor,                     // existing brightness/warmth/…
  contour / range / rhythmic_density: …,
  instrument: String,
}
```

### 3.2 The taste profile (what makes it *theirs*)
A student-owned profile of the music they love — genres, artists, films/scores, "I'm here because of Kendrick / Hans Zimmer / my church band." Captured at onboarding, editable, and **evolving** (the app learns which connections landed). This is the personalisation key and, frankly, a retention engine: the app feels like it *gets* you.
- Privacy: it's preference data, not sensitive like minors' audio — but for under-13 keep it minimal and parent-visible (reuse the Phase 3 consent posture). Store in the existing Supabase `profiles` neighbourhood.

### 3.3 The relevance generation (internet-connected LLM)
The engine (extends `crates/brain/src/coaching.rs`, same `HttpClient` seam) calls a current top-tier model **with web search / retrieval** (default to the latest Claude; provider-agnostic behind the trait). It receives the fingerprint + taste profile and returns, as structured JSON:
- **Connections**: 1–3 concrete links — *"the way you're laying behind the beat here is exactly the pocket D'Angelo built* Voodoo *on; it also shows up in the lo-fi you study to."* Each cites a **real** artist/recording.
- **A bridge to their world**: maps the technique to something on their taste list, so it never feels like museum music.
- **A "try this" hook**: a short, in-their-genre exercise that drills the same skill ("loop these two bars but pull them 30 ms later — that's the whole trick").

### 3.4 Anti-hallucination guardrails (the part that protects trust)
- **Grounding contract**: the prompt hands the model the fingerprint as the *sole* source of performance truth; a post-generation check rejects any response that asserts a key/tempo/groove the fingerprint didn't supply (or marked low-confidence).
- **Citations must resolve**: prefer web-search-backed references so artist/recording claims are real, not pattern-completed. Unverifiable specifics get dropped to genre-level statements.
- **Confidence-gated voice**: low feature confidence → the engine speaks in feels ("there's a bluesy, mixolydian-ish colour here") rather than false precision ("you are in G mixolydian").
- **Off the hot path entirely**: this is a *reflective* layer (end of a phrase group, in the recap, or on-demand "tell me what I just played") — never on the <25 ms mic-to-screen path. Latency and internet dependency live only in the optional, async coaching layer.

---

---

## 3B. Sub-track — Musical Language / Idiom Recognition (the "bebop test")

This is the concrete mechanism behind Track B's promise. "Relate it to my world" only lands if the system can actually *recognise the vocabulary* — "that line is a Parker-ism," "that's a ii–V bebop enclosure," "you quoted *Cry Me a River*" — not just report key/tempo. It's the hardest and highest-value part, and the one a trained ear will judge most harshly, so the grounding principle (§1) applies double.

### 3B.1 Two complementary engines

1. **Symbolic lick-matching (lead).** Transcribe the played line to notes (our monophonic pitch path already handles a single-line sax/trumpet solo; richer lines via the offline `crates/transcribe`), then match the note sequence against a **catalogue of known patterns** with melodic-similarity search (transposition-invariant, rhythm-aware). When a strong match hits, we can say something *specific and true*: "measure 3 — textbook bebop enclosure into the ii–V; here it is in *Now's the Time*." This is grounded (we point at the actual notes), explainable, and citable. It's the spiritual successor to the **Dig That Lick** / **Jazzomat** pattern-search work over the **Weimar Jazz Database** and the **Parker Omnibook**.[^djl][^jazzomat][^omnibook]
2. **Audio-embedding retrieval (augment).** Embed the audio with a music foundation model (e.g. **MERT**) and do nearest-neighbour retrieval against reference recordings/phrases.[^mert] This captures what transcription throws away — swing feel, articulation, timbre — and powers *flavour* statements: "your phrasing here sits near Stitt and early Parker." It is **never** used to assert a hard fact; only "reminds me of / shares a neighbourhood with."

### 3B.2 What we explicitly do *not* trust for hard claims

Audio-LLMs that "listen and describe" (Qwen2-Audio, MU-LLaMA, SALMONN, Audio Flamingo) are the most seductive option, but 2025 evaluations (MuChoMusic, MMAU, and pointedly *Investigating Modality Contribution in Audio LLMs for Music*) show they are **unreliable on fine musical perception and often answer from text priors rather than actually listening** — they will confidently mislabel an idiom.[^mucho][^modality] A perfect-pitch user catches that instantly → trust gone. So an audio-LLM may *phrase* a connection warmly, but the **claim itself must come from the symbolic matcher (specific licks) or be hedged retrieval (embeddings)** — never from the audio-LLM's say-so.

### 3B.3 Output shape (grounded + confidence-gated)

```
IdiomMatch {
  label: String,            // "Bebop enclosure", "Parker quote: Now's the Time"
  family: String,           // bebop / blues / modal / bossa / …
  note_span: (usize,usize), // which played notes — so we can point at them
  confidence: f32,          // strong match required to *name* a lick
  exemplars: Vec<Ref>,      // real recordings (named + linked, see licensing)
  source: Symbolic | Retrieval,  // hard claim vs "reminds me of"
}
```
A `Symbolic` match below the naming threshold degrades to a `Retrieval`-style "this has a bebop flavour"; below that, silence.

### 3B.4 Data & moat

- **Bootstrap on open assets:** Weimar Jazz Database (456 transcribed solos), the Dig That Lick DTL1000 pattern set (~1,700 solos), the Parker Omnibook. Enough to ship a credible jazz/bebop catalogue.[^djl][^jazzomat][^omnibook]
- **The moat is our own growing, labelled catalogue** of licks/idioms across genres (not just jazz), accumulated from real student sessions and teacher tagging over time. We do **not** train our own foundation model in Phase 4 (data/rights/cost) — we stand on open datasets + an off-the-shelf embedding model.
- **Reference-audio licensing:** v1 **names and links out** to recordings (Spotify/YouTube search), it does **not** host or stream clips. Embedding *reference* audio for retrieval is an internal index question to clear with counsel before launch.

### 3B.5 The validation spike comes first

Before committing an approach, **Idiom PR 0** is a research spike (this is the "spike vs doc" question resolved as *both*): assemble ~10 real solos (a few clear bebop lines, a blues, a modal tune, a couple of non-jazz controls) and evaluate, on the *same* clips:
- **Symbolic** (transcribe → Dig-That-Lick-style pattern match),
- **MERT embedding** retrieval, and
- **an audio-LLM** ("name the idiom"),
scoring each on **precision of idiom labels** (how often it's *right and specific*) and, critically, **false-confident rate** (how often it asserts a wrong idiom — the trust-killer). Deliverable: a short decision memo + a tiny labelled eval set we keep as a regression fixture. Only then do we build the chosen engine.

[^mert]: MERT: Acoustic Music Understanding Model — https://arxiv.org/html/2306.00107v4
[^djl]: Dig That Lick: Exploring Patterns in Jazz with Computational Methods — https://hal.science/hal-03084838/document
[^jazzomat]: The Jazzomat Research Project (Weimar Jazz Database) — https://jazzomat.hfm-weimar.de/
[^omnibook]: Reconstructing the Charlie Parker Omnibook via auto-transcription — https://arxiv.org/html/2405.16687
[^mucho]: MuChoMusic: Evaluating Music Understanding in Audio-Language Models — https://arxiv.org/pdf/2408.01337
[^modality]: Investigating Modality Contribution in Audio LLMs for Music — https://arxiv.org/pdf/2509.20641

---

## 4. Where it surfaces (UX)
- **Live (sparing):** an occasional connection card between phrases — not every phrase; rationed like the current tip queue.
- **Recap:** a "Your musical world today" section — the connections + one taste-matched thing to explore next.
- **On demand:** a "What was that?" button — student plays a lick, asks, gets the lineage. This is the shareable, demoable magic moment.

## 5. Architecture & guardrail-fit
- **Measurement in Rust** (`crates/theory`, `crates/tone`), thin JSON over IPC — business logic stays in the core (CLAUDE.md).
- **Relevance in the existing async coaching engine** — already an LLM-over-HTTP seam; add retrieval + the fingerprint/taste inputs. No new hot-path risk.
- **Coach, don't judge** holds: connections are encouragement and context, never scores.
- The learned **tone** model (Phase 3, dataset-gated) drops into the fingerprint when ready.

## 6. Testing
- **Track A is unit-testable against ground truth:** key detection accuracy on labelled audio fixtures; swing-ratio and tempo on synthetic/known grooves; intonation in cents on generated tones; onset/IOI accuracy. These are pass/fail, like the existing pitch tests.
- **Grounding tests (the critical ones):** given a fingerprint, assert the prompt exposes only supplied facts; a contract test that **rejects** a synthetic LLM response asserting an unsupported key/genre (mirrors how `coaching.rs` already tests prompt construction + fallback).
- **Personalisation tests:** taste profile changes change the connections; empty profile degrades gracefully to genre-level.
- **No-internet degradation:** retrieval down → fall back to model-only, genre-level, clearly hedged (never fabricated specifics).

## 7. PR slicing
- **PR 1 — `crates/theory`: key & mode detection** (A3). Smallest, unlocks the most. Free-play key in the recap.
- **PR 2 — Real intonation** (A1) + retire the `stability`-as-intonation proxy; tuning-tendency map.
- **PR 3 — Rhythm & groove** (A2): onset/IOI + tempo + swing + lay-back; `GrooveDescriptor`; retire the `1.0` placeholder.
- **PR 4 — `MusicalFingerprint` assembly** + the taste profile (schema + onboarding).
- **PR 5 — Relevance engine v1**: fingerprint + taste → LLM (no internet yet) → connections, with the grounding contract + tests.
- **PR 6 — Internet/retrieval grounding** + citation resolution + the "What was that?" moment.
- **PR 7 — Harmonic hints** (A4, best-effort) — stretch.

### Idiom Recognition sub-track (§3B) — runs after PR 4 (fingerprint), parallel to PR 5–6
- **Idiom PR 0 — validation spike**: symbolic vs MERT-embedding vs audio-LLM on ~10 real solos; score idiom-label precision + false-confident rate; decision memo + regression eval set. **Gates the rest of the sub-track.**
- **Idiom PR 1 — symbolic lick catalogue + matcher**: a pattern catalogue (bootstrap from Weimar Jazz DB / DTL1000 / Parker Omnibook) + transposition-invariant melodic-similarity match over the transcribed line; emits confidence-gated `IdiomMatch`. New crate (e.g. `crates/idiom`) or a `theory` module.
- **Idiom PR 2 — embedding retrieval**: MERT (or chosen model) nearest-neighbour "reminds me of" against a reference index; strictly hedged, never a hard claim.
- **Idiom PR 3 — surface it**: feed `IdiomMatch`es into the relevance engine + the "What was that?" moment, with named/linked exemplars (no hosted audio).

## 8. Cut lines (not in Phase 4)
- A trained **audio genre classifier** — we deliberately lean on theory features + LLM reasoning, not a learned genre model (honest about the difference; revisit if features prove insufficient).
- **Training our own music foundation model** — stand on open datasets + an off-the-shelf embedding model (MERT-class); our moat is the labelled idiom catalogue, not the encoder.
- **Audio-LLM as a source of hard idiom claims** — it may phrase, never assert (§3B.2); claims come from the symbolic matcher or hedged retrieval.
- **Hosting/streaming reference clips** — v1 names + links out only (§3B.4).
- **Real-time polyphonic** pitch (its own large effort; A4 rides on offline polyphony only).
- Auto-generated backing tracks / "play in the style of X" generation — adjacent product, later.
- Anything that puts the LLM or the internet on the real-time audio path.

## 9. Open questions for the founder
1. **Live vs reflective default** — is the connection card a (rationed) live thing, or recap-only for v1? (Proposal: recap + on-demand first; live later once it's tuned.)
2. **Model & retrieval** — which internet-capable model/tool for the relevance layer, and what's the per-session cost ceiling? (Proposal: latest Claude with web search, behind the existing provider-agnostic seam.)
3. **Taste onboarding depth** — quick "pick 5 artists" vs a richer survey? How much do we infer vs ask?
4. **How aggressive on rhythm feedback** — micro-timing "you're 25 ms behind" is powerful but can feel clinical; how much do we surface as numbers vs feel-language?
5. **Licensing/clips** — do we ever embed/stream reference snippets (rights!), or only *name* recordings and let the student go find them? (Proposal: name + link out for v1; no audio hosting.)
6. **Idiom genre coverage at launch** — start jazz/bebop only (where the open datasets are richest) and expand, or seed a few idioms across several genres from day one? (Proposal: jazz-deep first — it's the clearest "wow" and the data exists.)
7. **Building our own idiom catalogue** — when do we add teacher/student labelling so the catalogue (the moat) grows beyond the open datasets? Shapes whether idiom tagging is a Phase 4 or Phase 5 surface.

---

**End of design doc.**
