# Real-piano chord fixtures (#382)

Mixes of individually recorded real piano notes from the **University of
Iowa Electronic Music Studios Musical Instrument Samples** (Steinway & Sons
model B, fortissimo takes; freely provided by the University of Iowa for
any use, no restrictions). Single-note recordings were peak-normalized,
onset-trimmed, mixed with human-spread onsets (6–33 ms), and trimmed to
5 s mono 16-bit 44.1 kHz (single-e3: 7 s — the ring-out must fully clear).

Regeneration: `python3 scripts/gen-real-piano-fixtures.py <samples-dir>` —
the script carries every mix parameter; source notes fetched from
theremin.music.uiowa.edu.

These are the calibration ground truth for the chord pipeline — synthetic
renders (rich_piano_chords.rs) approximate physics, these ARE physics. The
VA's own piano recordings supersede them for calibration when available.
