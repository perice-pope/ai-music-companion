import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import SessionRecap from "./SessionRecap";
import { usePracticeStore } from "../stores/practiceStore";
import type { SessionRecap as RecapT } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

function fullRecap(overrides: Partial<RecapT> = {}): RecapT {
  return {
    overall_assessment: "You had a solid 15 minutes on trumpet.",
    strengths: ["Consistent tone.", "Steady tempo."],
    areas_to_improve: [
      "Intonation drifted sharp on high C.",
      "Rests sometimes feel cut short.",
    ],
    next_session_suggestions: [
      "Long tones with a drone.",
      "Slow tonguing exercises.",
    ],
    duration_secs: 900,
    phrase_count: 24,
    instrument: "Trumpet",
    ...overrides,
  };
}

function seedRecap(recap: RecapT | null, error: string | null = null) {
  usePracticeStore.setState({
    screen: "recap",
    status: "idle",
    sessionId: null,
    instrumentName: null,
    segmentId: null,
    startedAtMs: null,
    elapsedSecs: 0,
    phrases: [],
    tipQueue: [],
    recap,
    recapError: error,
    jamChart: null,
  });
}

describe("SessionRecap", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders strengths BEFORE areas-to-improve in DOM order", () => {
    seedRecap(fullRecap());
    render(<SessionRecap />);

    const strengths = screen.getByTestId("recap-strengths");
    const areas = screen.getByTestId("recap-areas");

    // Use compareDocumentPosition to assert DOM order, not visual
    // position. Math: DOCUMENT_POSITION_FOLLOWING (4) means `areas`
    // comes after `strengths`.
    const following =
      strengths.compareDocumentPosition(areas) &
      Node.DOCUMENT_POSITION_FOLLOWING;
    expect(following).toBeTruthy();
  });

  it("renders on a dark full-height surface in every variant (#186)", () => {
    // Light recap text must sit on a dark surface, not the browser's white.
    // Every branch (summary / error / empty) wraps in the same dark screen.
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    const summaryShell = screen.getByTestId("session-recap").parentElement;
    expect(summaryShell).toHaveClass("bg-gray-900");
    expect(summaryShell).toHaveClass("min-h-screen");

    seedRecap(null, "something went wrong");
    rerender(<SessionRecap />);
    expect(screen.getByTestId("session-recap").parentElement).toHaveClass(
      "bg-gray-900",
    );

    seedRecap(null);
    rerender(<SessionRecap />);
    expect(screen.getByTestId("session-recap").parentElement).toHaveClass(
      "bg-gray-900",
    );
  });

  it("renders the overall assessment and all three sections", () => {
    seedRecap(fullRecap());
    render(<SessionRecap />);
    expect(screen.getByTestId("recap-assessment").textContent).toContain(
      "solid 15 minutes",
    );
    screen.getByText("Consistent tone.");
    screen.getByText("Intonation drifted sharp on high C.");
    screen.getByText("Long tones with a drone.");
  });

  it("shows the tone read-out only when the recap carries a tone aggregate", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no tone panel.
    expect(screen.queryByTestId("recap-tone")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          tone: {
            brightness: 0.6,
            warmth: 0.5,
            air_noise: 0.2,
            core_clarity: 0.8,
            vibrato_quality: 0.55,
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    expect(screen.getByTestId("recap-tone")).toBeTruthy();
    screen.getByText("Brightness");
  });

  it("shows the detected key only when the recap carries one", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no key line.
    expect(screen.queryByTestId("recap-key")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          key: {
            tonic: 7,
            mode: "mixolydian",
            confidence: 0.82,
            margin: 0.1,
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    expect(screen.getByTestId("recap-key").textContent).toContain(
      "G Mixolydian",
    );
  });

  // #316: the key line states the claim only as firmly as the tracking
  // earned — leaning keys hedge, asserted/legacy keys stay flat, and a
  // session that never settled says so instead of guessing.
  it("hedges a leaning key and keeps asserted/legacy keys flat", () => {
    const key = {
      tonic: 8,
      mode: "ionian",
      confidence: 0.7,
      margin: 0.1,
    } as const;
    seedRecap(fullRecap({ fingerprint: { key, key_claim: "leaning" } }));
    const { rerender } = render(<SessionRecap />);
    expect(screen.getByTestId("recap-key").textContent).toContain(
      "leaning G# major toward the end",
    );

    // Asserted → today's flat form, no hedge.
    seedRecap(fullRecap({ fingerprint: { key, key_claim: "asserted" } }));
    rerender(<SessionRecap />);
    expect(screen.getByTestId("recap-key").textContent).toContain("G# major");
    expect(screen.getByTestId("recap-key").textContent).not.toContain(
      "leaning",
    );

    // Drifted (#404): the key carried the session but the strip wandered
    // off it by the close — a whole-session claim. "Toward the end" here
    // would contradict the strip the player watched (the VA's "leaning
    // F Phrygian" over a cycling E Locrian / F major close).
    seedRecap(fullRecap({ fingerprint: { key, key_claim: "drifted" } }));
    rerender(<SessionRecap />);
    expect(screen.getByTestId("recap-key").textContent).toContain(
      "mostly G# major — wandering by the end",
    );
    expect(screen.getByTestId("recap-key").textContent).not.toContain(
      "toward the end",
    );

    // Legacy recap (persisted before key_claim existed) → flat form, so old
    // sessions don't retroactively hedge.
    seedRecap(fullRecap({ fingerprint: { key } }));
    rerender(<SessionRecap />);
    expect(screen.getByTestId("recap-key").textContent).toContain("G# major");
    expect(screen.getByTestId("recap-key").textContent).not.toContain(
      "leaning",
    );
  });

  // #337 S4+S5: the score summary renders honest accuracy + worst
  // measures, and the RV bridge button rows a measure through 12 keys —
  // parking the exploration for the next session.
  it("renders the score summary and rows a worst measure through 12 keys", async () => {
    mockInvoke.mockResolvedValueOnce({
      label: "C · your 4-note cell · 3 roots · 60 BPM",
      music_xml: "<score-partwise/>",
      chips: [],
      root_pitch_classes: [0, 7, 2],
      root_names: ["C", "G", "D"],
      can_undo: false,
      staff: { fifths: 0, beats_per_measure: 4, total_beats: 4, notes: [] },
    });
    usePracticeStore.setState({
      activeScore: {
        id: "score-123",
        title: "Haydn",
        composer: null,
        source_filename: "haydn.mid",
        part_index: 0,
        duration_measures: 12,
      } as never,
    });
    seedRecap(
      fullRecap({
        score_summary: {
          score_title: "Haydn",
          judged: 10,
          accuracy_pct: 60,
          worst_measures: [
            { measure_number: 7, hit: 0, near: 0, missed: 2 },
            { measure_number: 3, hit: 0, near: 1, missed: 1 },
          ],
        },
      }),
    );
    render(<SessionRecap />);
    expect(screen.getByTestId("recap-score-summary")).toHaveTextContent(
      "Haydn — 60% of 10 notes clean",
    );
    expect(screen.getByTestId("worst-measure-7")).toHaveTextContent(
      "Measure 7 — 2 missed",
    );

    fireEvent.click(screen.getByTestId("row-measure-7"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("explore_measure", {
        scoreId: "score-123",
        measureNumber: 7,
      }),
    );
    // The exploration is parked for the next session and we head to the
    // selector — pendingExplore must survive returnToSelector.
    await waitFor(() => {
      const st = usePracticeStore.getState();
      expect(st.screen).toBe("selector");
      expect(st.pendingExplore?.label).toContain("your 4-note cell");
    });
  });

  // Kills review mutation (c): a parked exploration is PROMOTED into the
  // next session — startSession moves pendingExplore into explore.
  it("promotes pendingExplore into the next session", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "start_practice_session") return Promise.resolve("sess-1");
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    const dto = {
      label: "queued cell",
      music_xml: "<score-partwise/>",
      chips: [],
      root_pitch_classes: [0],
      root_names: ["C"],
      can_undo: false,
      staff: { fifths: 0, beats_per_measure: 4, total_beats: 4, notes: [] },
    };
    usePracticeStore.setState({
      pendingExplore: dto as never,
      status: "idle",
      screen: "selector",
    });
    await usePracticeStore.getState().startSession("trumpet");
    const st = usePracticeStore.getState();
    expect(st.explore?.label).toBe("queued cell");
    expect(st.pendingExplore).toBeNull();
  });

  // A calm bridge refusal must NOT vaporize the recap (review MUST-FIX 3):
  // the summary stays, an inline notice explains.
  it("keeps the recap when the bridge refuses, showing an inline notice", async () => {
    mockInvoke.mockRejectedValueOnce("measure 7 is all rests — nothing to row");
    usePracticeStore.setState({
      activeScore: { id: "score-1", title: "Haydn" } as never,
      bridgeNotice: null,
    });
    seedRecap(
      fullRecap({
        score_summary: {
          score_title: "Haydn",
          judged: 4,
          accuracy_pct: 75,
          worst_measures: [{ measure_number: 7, hit: 0, near: 0, missed: 1 }],
        },
      }),
    );
    render(<SessionRecap />);
    fireEvent.click(screen.getByTestId("row-measure-7"));
    await screen.findByTestId("bridge-notice");
    expect(screen.getByTestId("bridge-notice")).toHaveTextContent("all rests");
    // The recap and its summary are still standing.
    expect(screen.getByTestId("recap-score-summary")).toBeInTheDocument();
    expect(screen.getByTestId("recap-assessment")).toBeInTheDocument();
  });

  it("shows no score section for free-play recaps", () => {
    seedRecap(fullRecap());
    render(<SessionRecap />);
    expect(screen.queryByTestId("recap-score-summary")).toBeNull();
  });

  it("says the key kept moving only on an explicit unsettled claim", () => {
    // Tonal readings existed but never firmed into a claim → honest
    // "kept moving" copy instead of silence or a guess.
    seedRecap(
      fullRecap({
        fingerprint: {
          key_claim: "unsettled",
          tone: {
            brightness: 0.6,
            warmth: 0.5,
            air_noise: 0.2,
            core_clarity: 0.8,
            vibrato_quality: 0.55,
          },
        },
      }),
    );
    const { rerender } = render(<SessionRecap />);
    expect(screen.getByTestId("recap-key-unsettled").textContent).toContain(
      "kept moving",
    );
    expect(screen.queryByTestId("recap-key")).toBeNull();

    // A measured session with NO tonal readings (groove-only / percussive):
    // "kept moving" would claim motion nothing observed → no key copy.
    seedRecap(
      fullRecap({
        fingerprint: {
          tone: {
            brightness: 0.6,
            warmth: 0.5,
            air_noise: 0.2,
            core_clarity: 0.8,
            vibrato_quality: 0.55,
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    expect(screen.queryByTestId("recap-key-unsettled")).toBeNull();

    // No fingerprint at all (nothing measured) → no key copy of any kind.
    seedRecap(fullRecap());
    rerender(<SessionRecap />);
    expect(screen.queryByTestId("recap-key-unsettled")).toBeNull();
  });

  it("shows the intonation read-out only when the recap carries one", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no intonation line.
    expect(screen.queryByTestId("recap-intonation")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          intonation: {
            note_count: 24,
            mean_cents: 8,
            mean_abs_cents: 12,
            in_tune_ratio: 0.75,
            tendencies: [{ semitones_from_tonic: 4, mean_cents: 14, count: 6 }],
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    const line = screen.getByTestId("recap-intonation").textContent ?? "";
    expect(line).toContain("tends sharp");
    expect(line).toContain("major 3rd");
  });

  it("shows the groove read-out only when the recap carries one", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No fingerprint → no feel line.
    expect(screen.queryByTestId("recap-groove")).toBeNull();

    seedRecap(
      fullRecap({
        fingerprint: {
          groove: {
            tempo_bpm: 92,
            swing_ratio: 1.6,
            mean_ioi_secs: 0.4,
            timing_consistency: 0.95,
            onset_count: 40,
          },
        },
      }),
    );
    rerender(<SessionRecap />);
    const line = screen.getByTestId("recap-groove").textContent ?? "";
    expect(line).toContain("92 BPM");
    expect(line).toContain("swung");
    expect(line).toContain("steady");
  });

  it("shows the theory-grounded flavour line only when the recap carries one", () => {
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    // No flavour → no line (silence > lies). The placeholder idiom corpus
    // (#208) no longer drives this line.
    expect(screen.queryByTestId("recap-flavour")).toBeNull();

    // Explicit null is also silent.
    seedRecap(fullRecap({ flavour: null }));
    rerender(<SessionRecap />);
    expect(screen.queryByTestId("recap-flavour")).toBeNull();

    // A populated, hedged flavour (mode + swing) renders verbatim.
    seedRecap(
      fullRecap({
        flavour:
          "a modal-jazz feel — modal lines over a swung pulse (think Miles Davis)",
      }),
    );
    rerender(<SessionRecap />);
    const line = screen.getByTestId("recap-flavour").textContent ?? "";
    expect(line).toContain("Flavour:");
    expect(line).toContain("modal-jazz feel");
    expect(line).toContain("Miles Davis");
  });

  it("shows cross-genre connections only when the recap carries grounded ones", () => {
    // No connections → no panel (cold start / thin signal / silence).
    seedRecap(fullRecap());
    const { rerender } = render(<SessionRecap />);
    expect(screen.queryByTestId("recap-connections")).toBeNull();

    // Explicit empty array → still nothing (honest about "no grounded link").
    seedRecap(fullRecap({ connections: [] }));
    rerender(<SessionRecap />);
    expect(screen.queryByTestId("recap-connections")).toBeNull();

    // Grounded, hedged connection present → the quiet "In your world" panel.
    seedRecap(
      fullRecap({
        connections: [
          "the way you're laying back on the beat has the same pocket as a lot of the soul you love",
        ],
      }),
    );
    rerender(<SessionRecap />);
    const panel = screen.getByTestId("recap-connections");
    expect(panel).toBeTruthy();
    expect(panel.textContent).toContain("the same pocket");
  });

  it("empty-state recap (zero phrases) hides bullet sections but keeps buttons", () => {
    seedRecap(
      fullRecap({
        overall_assessment:
          "Looks like you didn't get to play this time — come back when you're ready.",
        strengths: [],
        areas_to_improve: [],
        next_session_suggestions: ["Just having the app running can help."],
        phrase_count: 0,
        duration_secs: 0,
      }),
    );
    render(<SessionRecap />);

    expect(screen.queryByTestId("recap-strengths")).toBeNull();
    expect(screen.queryByTestId("recap-areas")).toBeNull();
    screen.getByTestId("recap-next");
    // And crucially we still offer the user a way forward.
    screen.getByTestId("recap-practice-again");
    screen.getByTestId("recap-done");
    // data-variant reflects the mode so styling can differ later.
    expect(
      screen.getByTestId("session-recap").getAttribute("data-variant"),
    ).toBe("empty");
  });

  it("renders the fallback copy when recapError is set", () => {
    seedRecap(null, "llm timeout");
    render(<SessionRecap />);
    expect(
      screen.getByTestId("session-recap").getAttribute("data-variant"),
    ).toBe("error");
    // No strengths/areas sections in the error variant.
    expect(screen.queryByTestId("recap-strengths")).toBeNull();
    expect(screen.queryByTestId("recap-areas")).toBeNull();
    screen.getByText(/trouble generating your recap/i);
  });

  it("Practice again / Done both return to selector", () => {
    seedRecap(null, "llm timeout");
    const { rerender } = render(<SessionRecap />);

    fireEvent.click(screen.getByTestId("recap-practice-again"));
    expect(usePracticeStore.getState().screen).toBe("selector");
    expect(usePracticeStore.getState().recap).toBeNull();
    expect(usePracticeStore.getState().recapError).toBeNull();

    // Re-seed + test the Done button path independently.
    seedRecap(fullRecap());
    rerender(<SessionRecap />);
    fireEvent.click(screen.getByTestId("recap-done"));
    expect(usePracticeStore.getState().screen).toBe("selector");
  });

  // #349 T4a: a jam session's recap sketches the chord chart — timestamped
  // labels (m:ss, minute boundary included) with confidence dots,
  // unresolved stretches honest, privacy line present.
  it("sketches the jam chord chart when one exists", () => {
    seedRecap(fullRecap());
    usePracticeStore.setState({
      jamChart: [
        {
          label: "Cmaj7",
          root_pc: 0,
          quality: "maj7",
          confidence: 0.9,
          at_secs: 3.2,
          unresolved: false,
        },
        {
          label: "",
          root_pc: null,
          quality: null,
          confidence: 0,
          at_secs: 9.8,
          unresolved: true,
        },
        {
          label: "G7",
          root_pc: 7,
          quality: "dom7",
          confidence: 0.6,
          at_secs: 65.0,
          unresolved: false,
        },
      ],
    });
    render(<SessionRecap />);
    const chart = screen.getByTestId("recap-chord-chart");
    expect(chart).toHaveTextContent("What the room played");
    const chords = screen.getAllByTestId("chart-chord");
    expect(chords).toHaveLength(2);
    expect(chords[0]).toHaveTextContent("Cmaj7");
    expect(chords[0]).toHaveTextContent("0:03");
    // Minute boundary + zero padding: 65 s reads 1:05, never 1:5.
    expect(chords[1]).toHaveTextContent("1:05");
    // Dots reflect confidence: 0.9 → three, 0.6 → two.
    expect(chords[0]).toHaveTextContent("●●●");
    expect(chords[1].textContent).toContain("●●");
    // #390: the unresolved stretch reads as a muted span, not a "several
    // notes…" row. A lone reading spans 0 s, so no duration is claimed —
    // the 55 s until the G7 may have been mostly rest.
    const unresolved = screen.getByTestId("chart-unresolved");
    expect(unresolved).toHaveTextContent("· · ·");
    expect(unresolved).not.toHaveTextContent("unclear");
    expect(unresolved).toHaveTextContent("0:09");
    expect(chart).toHaveTextContent("nothing was recorded");
  });

  it("shows no chart section for a normal session", () => {
    seedRecap(fullRecap());
    usePracticeStore.setState({ jamChart: null });
    render(<SessionRecap />);
    expect(screen.queryByTestId("recap-chord-chart")).toBeNull();
  });

  // #390 AC: 50 consecutive unresolved entries collapse to ONE rendered row
  // — the wall of "several notes…" is gone; the real chords still frame it.
  it("collapses a run of unresolved entries into one span with its duration", () => {
    seedRecap(fullRecap());
    const unresolvedRun = Array.from({ length: 50 }, (_, k) => ({
      label: "",
      root_pc: null,
      quality: null,
      confidence: 0,
      at_secs: 12 + k * 2,
      unresolved: true,
    }));
    usePracticeStore.setState({
      jamChart: [
        {
          label: "Am7",
          root_pc: 9,
          quality: "min7",
          confidence: 0.8,
          at_secs: 10,
          unresolved: false,
        },
        ...unresolvedRun,
        {
          label: "Dm",
          root_pc: 2,
          quality: "min",
          confidence: 0.7,
          at_secs: 132,
          unresolved: false,
        },
      ],
    });
    render(<SessionRecap />);
    const spans = screen.getAllByTestId("chart-unresolved");
    expect(spans).toHaveLength(1);
    // The run's readings span 12 s → 110 s = 98 s → rounds to "2m"
    // (never floors to "1m"); the silent gap to the Dm at 132 s is
    // NOT billed as unclear time.
    expect(spans[0]).toHaveTextContent("0:12");
    expect(spans[0]).toHaveTextContent("(2m unclear)");
    expect(screen.getAllByTestId("chart-chord")).toHaveLength(2);
  });

  // #390: span durations are the observed readings only — honest at the
  // seconds/minutes boundary, and a lone blip claims no duration at all
  // (the recorder writes nothing during silence, so anything past the
  // last reading may be rest).
  it("reports observed span durations honestly across the s/m boundary", () => {
    const run = (from: number, to: number, stepSecs: number) =>
      Array.from(
        { length: Math.floor((to - from) / stepSecs) + 1 },
        (_, k) => ({
          label: "",
          root_pc: null,
          quality: null,
          confidence: 0,
          at_secs: from + k * stepSecs,
          unresolved: true,
        }),
      );
    const chord = (label: string, at_secs: number) => ({
      label,
      root_pc: 0,
      quality: "maj",
      confidence: 0.9,
      at_secs,
      unresolved: false,
    });
    seedRecap(fullRecap({ duration_secs: 600 }));
    usePracticeStore.setState({
      jamChart: [
        // 45 s of readings → seconds form, never "0m".
        ...run(10, 55, 5),
        chord("C", 60),
        // 75 s of readings → "1m", never "75s" (boundary is 60 s).
        ...run(100, 175, 5),
        chord("F", 200),
        // A lone trailing blip, then silence to session end (600 s):
        // dots only — two minutes of rest never read "(2m unclear)".
        ...run(230, 230, 5),
      ],
    });
    render(<SessionRecap />);
    const spans = screen.getAllByTestId("chart-unresolved");
    expect(spans).toHaveLength(3);
    expect(spans[0]).toHaveTextContent("(45s unclear)");
    expect(spans[1]).toHaveTextContent("(1m unclear)");
    expect(spans[2]).toHaveTextContent("· · ·");
    expect(spans[2]).not.toHaveTextContent("unclear");
  });

  // #390: a session that resolved nothing gets one honest line, not a wall.
  it("replaces an all-unresolved chart with one honest line", () => {
    seedRecap(fullRecap());
    usePracticeStore.setState({
      jamChart: Array.from({ length: 40 }, (_, k) => ({
        label: "",
        root_pc: null,
        quality: null,
        confidence: 0,
        at_secs: k * 5,
        unresolved: true,
      })),
    });
    render(<SessionRecap />);
    expect(screen.getByTestId("chart-all-unclear")).toHaveTextContent(
      "nothing settled long enough to name",
    );
    expect(screen.queryAllByTestId("chart-unresolved")).toHaveLength(0);
    expect(screen.queryAllByTestId("chart-chord")).toHaveLength(0);
    // The privacy line survives the honest-line branch.
    expect(screen.getByTestId("recap-chord-chart")).toHaveTextContent(
      "nothing was recorded",
    );
  });

  // #390: discoverability — the sketch is a real section (heading weight of
  // "Tone today") and sits with the session read-outs, before Strengths,
  // instead of below the fold.
  it("gives the chart a section heading and renders it before Strengths", () => {
    seedRecap(fullRecap());
    usePracticeStore.setState({
      jamChart: [
        {
          label: "Cmaj7",
          root_pc: 0,
          quality: "maj7",
          confidence: 0.9,
          at_secs: 3.2,
          unresolved: false,
        },
      ],
    });
    render(<SessionRecap />);
    const heading = screen.getByRole("heading", {
      level: 3,
      name: "What the room played",
    });
    expect(heading).toHaveClass("uppercase");
    const chart = screen.getByTestId("recap-chord-chart");
    const strengths = screen.getByTestId("recap-strengths");
    const following =
      chart.compareDocumentPosition(strengths) &
      Node.DOCUMENT_POSITION_FOLLOWING;
    expect(following).toBeTruthy();
  });
});
