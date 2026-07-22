import { describe, expect, it } from "vitest";
import {
  describeLateSync,
  describeSession,
  describeSplitDay,
  flagRetryFarming,
  flagSessionSplitting,
  flagWalkAway,
  lateSyncAnnotations,
  toolOnNoNoteSpans,
  toolSpans,
} from "./integrity";

// The fudge-vector table (datamodel §5) is the test spec: each metric must
// fire above its bar and stay quiet below it — both directions, per spec
// AC8. Bars are pinned in docs/specs/449-t4-dashboard.md §4.

describe("F1 walk-away (flagWalkAway)", () => {
  it("fires above the bar: long wall, under a minute of sound, near-total silence", () => {
    expect(
      flagWalkAway({ wall_min: 30, played_min: 0.5, silence_ratio: 0.97 }),
    ).toBe(true);
  });

  it("stays quiet below the bar", () => {
    // Real playing time.
    expect(
      flagWalkAway({ wall_min: 30, played_min: 4, silence_ratio: 0.85 }),
    ).toBe(false);
    // Short session — a quick touch is not a walk-away.
    expect(
      flagWalkAway({ wall_min: 5, played_min: 0.5, silence_ratio: 0.95 }),
    ).toBe(false);
    // Silence just under the bar.
    expect(
      flagWalkAway({ wall_min: 30, played_min: 0.5, silence_ratio: 0.89 }),
    ).toBe(false);
  });

  it("never nominates on missing data (honest absence, not zeros)", () => {
    expect(
      flagWalkAway({ wall_min: 30, played_min: null, silence_ratio: null }),
    ).toBe(false);
  });
});

describe("F4 retry-farming (flagRetryFarming)", () => {
  it("fires at 6 retries of one (material, tonic)", () => {
    expect(flagRetryFarming(6)).toBe(true);
    expect(flagRetryFarming(12)).toBe(true);
  });

  it("stays quiet at 5 and on missing data", () => {
    expect(flagRetryFarming(5)).toBe(false);
    expect(flagRetryFarming(null)).toBe(false);
  });
});

describe("F6 session-splitting (flagSessionSplitting)", () => {
  it("fires on many sessions with slivers of sound each", () => {
    // 4 sessions, 1 min of sound each = 60 s/session < 120 s bar.
    expect(flagSessionSplitting({ sessions: 4, played_min: 4 })).toBe(true);
  });

  it("stays quiet below the bar", () => {
    // Two sessions is a normal day, however short.
    expect(flagSessionSplitting({ sessions: 2, played_min: 1 })).toBe(false);
    // Many sessions but each has real playing (5 min apiece).
    expect(flagSessionSplitting({ sessions: 4, played_min: 20 })).toBe(false);
    // No played data: absence, not evidence.
    expect(flagSessionSplitting({ sessions: 6, played_min: null })).toBe(false);
  });
});

describe("F8 tool-on-no-notes (toolSpans / toolOnNoNoteSpans)", () => {
  const ev = (kind: string, at_secs: number, session_id = "s1") => ({
    session_id,
    at_secs,
    kind,
    params: {},
  });
  const phrase = (
    start_secs: number,
    end_secs: number,
    note_count: number,
    session_id = "s1",
  ) => ({ session_id, phrase_index: 0, start_secs, end_secs, note_count });

  it("fires when the metronome ran ≥60s over zero detected notes", () => {
    const spans = toolOnNoNoteSpans(
      [ev("pocket_start", 0), ev("pocket_stop", 300)],
      [],
      "s1",
    );
    expect(spans).toHaveLength(1);
    expect(spans[0]).toMatchObject({ start_secs: 0, end_secs: 300 });
  });

  it("stays quiet when notes were detected inside the span", () => {
    expect(
      toolOnNoNoteSpans(
        [ev("pocket_start", 0), ev("pocket_stop", 300)],
        [phrase(50, 90, 42)],
        "s1",
      ),
    ).toHaveLength(0);
  });

  it("stays quiet on spans shorter than the 60s bar", () => {
    expect(
      toolOnNoNoteSpans([ev("band_start", 0), ev("band_stop", 30)], [], "s1"),
    ).toHaveLength(0);
  });

  it("closes an unterminated span at the last known event, never beyond the evidence", () => {
    const spans = toolSpans(
      [ev("pocket_start", 100), ev("pocket_tempo", 400)],
      "s1",
    );
    expect(spans).toEqual([
      { session_id: "s1", start_secs: 100, end_secs: 400 },
    ]);
  });

  it("keeps sessions separate (one clock per session)", () => {
    const spans = toolOnNoNoteSpans(
      [ev("pocket_start", 0, "s1"), ev("pocket_stop", 300, "s1")],
      [phrase(0, 300, 50, "s2")], // notes in ANOTHER session don't excuse s1
      "s1",
    );
    expect(spans).toHaveLength(1);
  });
});

describe("F9 late sync (lateSyncAnnotations)", () => {
  const row = (started_at: string, created_at: string, student_id = "s1") => ({
    session_id: `${started_at}-${student_id}`,
    student_id,
    started_at,
    created_at,
  });

  it("annotates sessions that arrived 48h or more after they were played", () => {
    const annotations = lateSyncAnnotations([
      row("2026-05-01T16:00:00Z", "2026-06-03T10:00:00Z"),
      row("2026-05-30T16:00:00Z", "2026-06-03T10:00:00Z"),
    ]);
    expect(annotations).toHaveLength(1);
    expect(annotations[0]).toMatchObject({
      student_id: "s1",
      syncedOn: "2026-06-03",
      playedFrom: "2026-05-01",
      playedTo: "2026-05-30",
      sessionCount: 2,
    });
    // The copy shows the gap, it does not judge it.
    expect(describeLateSync(annotations[0])).toBe(
      "Synced 2026-06-03, played 2026-05-01 to 2026-05-30",
    );
  });

  it("stays quiet for prompt syncs (under 48h)", () => {
    expect(
      lateSyncAnnotations([
        row("2026-07-01T10:00:00Z", "2026-07-03T09:00:00Z"),
      ]),
    ).toHaveLength(0);
  });
});

describe("calm copy (evidence phrasing, never verdicts)", () => {
  it("describes the wall-vs-played gap as two facts", () => {
    expect(describeSession(42, 3)).toBe("42 min open, 3 min of sound");
    expect(describeSession(42, 0.5)).toBe(
      "42 min open, under a minute of sound",
    );
  });

  it("renders missing numbers as absence, not zero", () => {
    expect(describeSession(null, null)).toBe("—, played time not synced");
  });

  it("split-day copy states counts, not conclusions", () => {
    const text = describeSplitDay({
      student_id: "s1",
      display_name: "Ava",
      date_key: "2026-07-20",
      sessions: 5,
      playedMinPerSession: 1.2,
    });
    expect(text).toBe("5 short sessions this day, about 1.2 min of sound each");
  });
});
