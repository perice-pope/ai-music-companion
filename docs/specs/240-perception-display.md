# Spec: Perception display + honest key ambiguity + Bluetooth warning (#240)

## 1. Summary
Make the adaptive engine **legible**: a live in-session panel that shows what the app
hears (tempo, feel, key + confidence), presents the key **honestly** (best guess + the
relative-key alternative, with confirm/lock/override), and warns when the audio **output is
Bluetooth** (which drops the band out while the mic is live). Sets up the Random Variation
epic, where the same perception decides which variation to deal.

## 2. Problem / why
First live test of #212: hitting "Play with me" is a black box — no sense of tempo/key/feel
or what to expect; the key is asserted (a G triad → "G major", but it could be E minor); and
on a Bluetooth speaker the band only plays once you stop (confirmed: Bluetooth output starves
under live capture). Source: user feedback 2026-06-17 (a music master's grad).

## 3. Non-goals
- The Random Variation engine itself (#C epic) — this only surfaces perception + key control.
- A full audio-device router/picker (a one-line output warning is in scope; full picker later).
- Changing the detection algorithms (KeyTracker / LiveClock) — we surface what they already compute.

## 4. Contract / interface
### Backend — `brain::perception` (new)
```rust
pub struct PerceptionTracker { /* LiveClock + theory::KeyTracker */ }
impl PerceptionTracker {
    pub fn new() -> Self;
    pub fn observe(&mut self, event: &ears::AudioEvent); // onset → clock; pitch → key
    pub fn snapshot(&mut self, now_secs: f64) -> PerceptionSnapshot;
    pub fn reset(&mut self);
}
#[derive(Serialize)] pub struct PerceptionSnapshot {
    pub tempo_bpm: Option<f32>,
    pub swing_ratio: Option<f32>,
    pub locked: bool,            // clock locked (a real pulse)
    pub key: Option<KeySnapshot>,
}
#[derive(Serialize)] pub struct KeySnapshot {
    pub tonic: u8, pub mode: String, // serialized mode
    pub name: String,                // "G major"
    pub confidence: f32,
    pub alternative: Option<String>, // relative key, e.g. "E minor"
}
```
Keeps `groove`/`theory` out of the Tauri crate (mirrors the S4a driver pattern).

### App — new Tauri event + command
- Event `perception` (`PerceptionSnapshot`), emitted ~8 Hz from the pipeline worker's
  audio-event path during a session.
- Command `set_accompaniment_key(tonic: u8, minor: bool)` / `clear_accompaniment_key()` — pin
  or release a user-chosen key on the band (overrides auto-detect). (Slice 2.)

### Frontend
- `PerceptionPanel` (subscribes to `perception`): "🎧 ~92 BPM · feel · **G major** (or E minor?)"
  with a listening/locked cue and confidence. Key chips → confirm / pick / lock. (Override = slice 2.)
- Output warning near "Play with me": Bluetooth/external output can drop out; prefer built-in/wired.

## 5. Acceptance criteria (numbered, testable)
1. Given a steady pulse, `PerceptionTracker::snapshot` reports tempo within ±5% and `locked = true`.
2. Given pitches forming a clear key, the snapshot's `key.name` matches and `alternative` is the
   relative key (G major → "E minor"; A minor → "C major").
3. With too few onsets, `tempo_bpm = None`, `locked = false` (no fabricated pulse).
4. The `perception` event reaches the frontend and the panel renders live tempo + key + a
   listening/locked state that updates as events arrive.
5. (Slice 2) Overriding the key pins the band to the chosen key; clearing returns to auto.
6. (Slice 1) A Bluetooth/non-built-in output shows a calm warning; built-in shows none (or a tip).
7. Offline — no network; perception runs on the processing thread; IPC throttled (~8 Hz).

## 6. Edge cases
- Silence → snapshot all-None/false (AC3). Key below the tracker's confidence floor (0.4) isn't
  reported at all (`key = None`). When a key *is* reported but confidence is low (< ~0.55), it's
  shown **tentatively** ("maybe G major"); the relative **alternative is always offered** — it's the
  uncertainty cue, most useful exactly when we're unsure (so it is *not* suppressed on low confidence).
- Mode→relative mapping covers all 7 modes' relative major/minor (Ionian↔Aeolian classic; for the
  others, the relative is the parallel Ionian/Aeolian sharing the key signature — document the choice).
- Output device unknown / non-macOS → no false Bluetooth claim (warn generically or stay silent).

## 7. Test plan
| AC | Test | Asserts |
|---|---|---|
| AC1 | `brain::perception::tests::tempo_locks_on_steady_onsets` | tempo ±5%, locked |
| AC2 | `brain::perception::tests::key_name_and_relative_alternative` | name + relative alt |
| AC3 | `brain::perception::tests::silence_is_unlocked_no_tempo` | None/false |
| AC4 | `App.test` perception listener + `PerceptionPanel.test` | event → store → render |
| AC5 | `commands::tests::set/clear_accompaniment_key` + store/component (slice 2) | key pinned/released |
| AC6 | `PerceptionPanel.test` bluetooth warning shown/hidden | warning logic |
| AC7 | disclosure check; no-alloc not required (processing thread, alloc OK) | no egress |

## 8. Slice breakdown
1. **Slice 1 — perception panel + output warning.** `brain::perception` + `perception` event +
   `PerceptionPanel` (read-only "here's what I hear") + Bluetooth output tip. Satisfies AC1–4, 6, 7.
2. **Slice 2 — key confirm/override/lock.** `set/clear_accompaniment_key` commands wired to the
   accompaniment driver; key chips become interactive. Satisfies AC5.

## 9. Risks
- Live key from a parallel `KeyTracker` in `brain::perception` may differ slightly from the
  aggregator's per-phrase key — acceptable (both are estimates); keep one config.
- "Feel"/swing wording — keep honest ("straight" / "swung" / blank when unsure).

## 10. References
#240, #212, `crates/groove/src/clock.rs`, `crates/theory/src/{tracker,key,lib}.rs`,
`apps/desktop/src-tauri/src/commands.rs` (audio-event closure), `apps/desktop/src/components`.
