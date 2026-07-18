#!/usr/bin/env python3
"""Regenerate crates/brain/tests/fixtures/real_piano/*.wav (#382).

Sources: University of Iowa Electronic Music Studios MIS piano samples
(Steinway model B, fortissimo), freely provided for any use. Download the
needed Piano.ff.<NOTE>.aiff files from
  https://theremin.music.uiowa.edu/sound%20files/MIS/Piano_Other/piano/
convert to mono 16-bit 44.1k wav (macOS: afconvert -f WAVE -d LEI16@44100
-c 1 in.aiff out.wav), place them in a directory, and run:
  python3 scripts/gen-real-piano-fixtures.py <samples-dir>
"""
import sys, wave, struct, os

SR = 44100
SRC = sys.argv[1]
DST = os.path.join(os.path.dirname(__file__), "..", "crates/brain/tests/fixtures/real_piano")

def load(note):
    with wave.open(f"{SRC}/Piano.ff.{note}.wav", "rb") as w:
        assert (w.getframerate(), w.getnchannels(), w.getsampwidth()) == (SR, 1, 2)
        raw = w.readframes(w.getnframes())
    x = list(struct.unpack(f"<{len(raw)//2}h", raw))
    peak = max(abs(v) for v in x) or 1
    i0 = next(i for i, v in enumerate(x) if abs(v) > peak * 0.01)
    x = x[max(0, i0 - 441):]  # keep 10ms pre-attack
    g = 0.5 * 32767 / peak    # peak-normalize each note
    return [v * g for v in x]

def mix(name, notes, secs, offsets_ms):
    n = int(secs * SR)
    out = [0.0] * n
    for note, off in zip(notes, offsets_ms):
        x = load(note)
        o = int(off * SR / 1000)
        for i in range(min(len(x), n - o)):
            out[o + i] += x[i]
    peak = max(abs(v) for v in out) or 1
    g = 0.85 * 32767 / peak
    fade = int(0.05 * SR)
    for i in range(fade):
        out[n - fade + i] *= 1.0 - i / fade
    data = struct.pack(f"<{n}h", *(int(max(-32767, min(32767, v * g))) for v in out))
    with wave.open(f"{DST}/{name}.wav", "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(data)
    print(name, os.path.getsize(f"{DST}/{name}.wav") // 1024, "KB")

mix("c-major", ["C4", "E4", "G4"], 5.0, [0, 12, 22])
mix("g7", ["G3", "B3", "D4", "F4"], 5.0, [0, 10, 18, 26])
mix("c-over-e", ["E3", "C4", "E4", "G4"], 5.0, [0, 14, 24, 32])
mix("single-e3", ["E3"], 7.0, [0])  # 7s: the ring-out must fully clear
mix("cluster", ["B3", "C4", "Db4", "D4", "E4", "F4", "Gb4"], 5.0, [0, 6, 11, 17, 22, 28, 33])
# Review r5 MF3: LH root + RH triad — the most common amateur voicing; the
# bass sits 12/19/28 semitones under the triad tones, exactly where the
# harmonic subtraction rows land. Pins the bleed weights against reality.
mix("c-major-over-c3", ["C3", "C4", "E4", "G4"], 5.0, [0, 10, 19, 28])
