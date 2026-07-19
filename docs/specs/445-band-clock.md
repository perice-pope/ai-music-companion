# Spec: Play With Me locks to the Pocket's clock (#445 pt 9)

## 1. Summary
Founder: "Play With Me should now also lock in to the metronome —
whatever mode it's in :D". The #349 one-clock rule extended to the
band: click and band can never disagree about tempo.

## 2. Found reality
- Band and Pocket are **mutually exclusive audio owners**:
  `start_accompaniment` tears the pocket down (commands.rs) and
  `start_pocket` tears the band down, both under
  `accompaniment_cmd_lock`. One output device, one owner — they never
  sound together.
- The band's tempo today comes from its **own** `LiveClock`
  (`AccompanimentDriver::observe_event` → `ClockState` → synth,
  brain/accompaniment.rs) — a second, raw, unclamped clock with no
  relation to the Pocket's set/followed/frozen tempo. Two clocks that
  can disagree: the exact violation the founder is naming.
- The Pocket's follow policy (locked + 40–220 range + ≥2 BPM delta +
  ≥1 s throttle + handoff freeze after `HANDOFF_FOLLOW_MS`) lives in
  `practiceStore.setPerception`; its backend seam is
  `set_pocket_tempo` → same-clamp SPSC push → the phase-preserving
  retime in `TempoFedMetronome`.

## 3. Chosen design — the band becomes the clock carrier
Because the band REPLACES the click, "lock to the metronome" cannot
mean listening to a running click (there is none). The band must carry
the same effective clock the click would:
- `start_accompaniment` gains `tempo_bpm: Option<f64>` (the frontend
  passes the Pocket's set tempo), clamped by the SAME
  `clamp_pocket_params`, and installed as a **tempo override** on the
  synth through the existing SPSC control channel
  (`AccompanimentControl::SetTempo`). The clamp+forward lives in ONE
  seam (`set_clamped_band_tempo`, review MF1 — the band's
  `push_clamped_tempo`) under both the start-time install and
  `set_band_tempo`, pinned by a channel-drain test.
- **The two-clock rule (review MF2).** Solo practice: the Pocket's
  clock carries — override installed, band plays immediately. Room
  mode ("listen to the room"): the room's live players ARE the clock —
  `tempo_bpm: null`, NO override installed, the band keeps the legacy
  listen-and-join path (silent until the live clock locks onto the
  room's pulse, aligned to its phase), and the follow policy never
  streams `set_band_tempo` at it.
- With an override the band **plays immediately** at that tempo —
  anchor semantics. (A band that still waited for the player's lock
  would leave them with no clock at all, since starting it silences
  the click.) Live-lock grid realignment is suspended while
  overridden: the band IS the clock; the player locks to it.
- New `set_band_tempo` command — the band's `set_pocket_tempo`: same
  clamp, calm no-op when silent, SPSC into the render source, and the
  retime is phase-preserving (`beat_pos` never reset).
- Frontend: **one follow policy, two carriers.** `setPerception`'s
  existing gate targets the click when the pocket plays and the band
  when Play With Me plays (never both — the backend enforces
  exclusivity). Anchor: no stream. Follow: the clamped/throttled
  stream. Handoff: follows, then freezes and the stream stops. A
  fresh band start resets the follow life (the click's MF4
  discipline).
- Without an override (room mode, or any caller passing `None`) the
  synth behaves exactly as before: silent until the live clock locks.

## 4. ACs
1. A tempo override makes the band play immediately at that tempo with
   NO live lock (anchor semantics), and `start_accompaniment` installs
   the clamped set tempo through the channel.
2. Retiming an overridden band is phase-preserving: the beat grid is
   never reset, the new rate takes effect; `set_band_tempo` clamps by
   the same 40–220 rule and is a calm no-op when no band is playing.
3. While overridden, a live perception lock neither realigns the
   band's grid nor changes its tempo — the band owns the clock.
4. With the band playing, follow mode streams `set_band_tempo` under
   the SAME gates as the click (locked, 40–220, ≥2 BPM delta, ≥1 s
   throttle); anchor streams nothing; handoff follows then freezes and
   the stream stops.
5. One clock, one carrier: the band carrier never fires
   `set_pocket_tempo`; a fresh band start resets frozen/last-sent
   state so a stale follow life can't leak in; `start_accompaniment`
   carries the Pocket's set tempo.
6. No regression: without an override the synth stays silent until the
   live clock locks (existing tests), and the render/control path
   stays allocation-free including `SetTempo` drains.
7. Clamp seam (review MF1): 250 → 220, 20 → 40, NaN → 40 arrive ON THE
   CHANNEL through the band's clamp seam — dropping the clamp is a
   killed mutant, not a survivable one.
8. Room mode (review MF2): `start_accompaniment(None)` pushes no
   `SetTempo`; the frontend sends `tempoBpm: null` when `listenToRoom`
   is active and never streams `set_band_tempo` while it is.

## 5. Test map
| AC | Test |
|---|---|
| 1 | brain: `tempo_override_plays_immediately_without_live_lock`; `set_tempo_reaches_synth_through_channel` (the start_accompaniment seam) |
| 2 | brain: `override_retime_is_phase_preserving`; commands: `set_band_tempo_is_a_noop_when_silent` + shared-clamp pin in `pushed_tempos_arrive_clamped`/`clamp` table |
| 3 | brain: `live_lock_does_not_realign_grid_under_override` |
| 4 | store: band follow gates test, anchor-silent test, handoff-freeze test |
| 5 | store: carrier-exclusivity test, fresh-band-follow-life test, start payload test |
| 6 | brain: existing silent-until-lock suite (unchanged); alloc test extended with `SetTempo` pushes |
| 7 | commands: `band_tempos_arrive_clamped_at_the_channel` |
| 8 | commands: `install_band_clock_room_mode_installs_no_override`; store: "room mode starts the band with no override and streams nothing to it" |

## 6. Follow-up polish (deferred, disclosed)
- The band has no handoff **drift line** ("holding your N / +X BPM
  ahead") like PocketControl's — the freeze itself works for the band
  carrier; the honest drift readout is UI polish for a later slice.
