import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";

/**
 * #453 S3 — the coaching box: surfaces AT MOST ONE history-grounded
 * suggestion, HOLDS through empty results (rule 0), replaces in place,
 * a dismissal quiets it for the session, and it wears the calm muted
 * violet — never the reveal's amber alarm. No suggestion → no chrome.
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import CoachingBox from "./CoachingBox";
import { usePracticeStore } from "../stores/practiceStore";

const trend = {
  kind: "trend",
  text: "Your Eb major rows are sitting at 54% over 6 attempts (last one 2 days ago) — worth a slow pass.",
  evidence:
    "key_mastery 3:major: 6 attempts, accuracy EWMA 0.54, last attempt 2d ago",
};
const momentum = {
  kind: "momentum",
  text: "Your 3-note cell (0 4 7) climbed from 50% to 80% across its last 8 graded rows — push the tempo.",
  evidence: "cell [0 4 7]: older-half mean 0.50 → newer-half mean 0.80",
};
// #454 S3: the box's second voice — a method-book tip with its attribution.
const schlossberg = {
  topic: "Long tones and pitch stability",
  guidance:
    "There are drills for exactly this in Schlossberg's Daily Drills — start the note softly, let it grow, and keep the pitch absolutely level.",
  source_line: "Max Schlossberg, Daily Drills and Technical Studies",
};

/** Route the two coaching fetches by command name. */
function routeCoachingFetches(suggestions: unknown, tip: unknown): void {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "practice_suggestions") return Promise.resolve(suggestions);
    if (cmd === "method_book_tip") return Promise.resolve(tip);
    return Promise.resolve(null);
  });
}

beforeEach(() => {
  mockInvoke.mockReset();
  usePracticeStore.setState({
    coachingSuggestion: null,
    coachingTip: null,
    coachingQuieted: false,
    // The box shares a column with the reveal — keep the reveal empty so
    // the no-empty-chrome assertion below means what it says.
    revealQueue: [],
  });
});

describe("CoachingBox (#453 S3)", () => {
  // AC8: with nothing to say (and the reveal empty too) the box renders
  // NOTHING — no empty chrome. Fails if an empty-state shell appears.
  it("renders nothing when history has nothing to say", () => {
    const { container } = render(<CoachingBox />);
    expect(screen.queryByTestId("coaching-box")).toBeNull();
    expect(container.innerHTML).toBe("");
  });

  // AC5: the store's fetch is routed by command name and the box surfaces
  // ONLY the first pinned suggestion. Fails if the wire name drifts or a
  // second suggestion leaks into the surface.
  it("a fetched suggestion surfaces — first only, routed by command name", async () => {
    mockInvoke.mockResolvedValueOnce([trend, momentum]);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(mockInvoke).toHaveBeenCalledWith("practice_suggestions", undefined);
    expect(screen.getByTestId("coaching-box-text").textContent).toContain(
      "Eb major",
    );
    expect(screen.getAllByTestId("coaching-box")).toHaveLength(1);
    expect(screen.queryByText(/climbed from/)).toBeNull();
  });

  // AC6 (rule 0): an EMPTY analyzer result never clears the shown
  // suggestion — the box holds. Fails if empties start clearing.
  it("holds through an empty fetch — rule 0", async () => {
    mockInvoke.mockResolvedValueOnce([trend]);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    mockInvoke.mockResolvedValueOnce([]);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(screen.getByTestId("coaching-box-text").textContent).toContain(
      "Eb major",
    );
  });

  // AC6 (rule 0): a newer suggestion replaces in place — one box, new text.
  it("a newer suggestion replaces in place", async () => {
    mockInvoke.mockResolvedValueOnce([trend]);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    mockInvoke.mockResolvedValueOnce([momentum]);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(screen.getAllByTestId("coaching-box")).toHaveLength(1);
    expect(screen.getByTestId("coaching-box-text").textContent).toContain(
      "climbed from 50% to 80%",
    );
  });

  // AC6: dismissal quiets the box for the session — later fetches with
  // fresh suggestions stay quiet. Fails if dismiss only hides once.
  it("dismissal quiets the box for the session", async () => {
    mockInvoke.mockResolvedValueOnce([trend]);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    fireEvent.click(screen.getByTestId("coaching-box-dismiss"));
    expect(screen.queryByTestId("coaching-box")).toBeNull();
    mockInvoke.mockResolvedValueOnce([momentum]);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(screen.queryByTestId("coaching-box")).toBeNull();
  });

  // AC7: the calm palette pin — muted violet family, and NO amber class
  // anywhere in the box (the alarm palette stays the reveal's). Fails if
  // the box drifts toward the reveal's colors.
  it("calm palette: muted violet, never the amber alarm", async () => {
    mockInvoke.mockResolvedValueOnce([trend]);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    const box = screen.getByTestId("coaching-box");
    expect(box.className).toContain("bg-violet-950/40");
    expect(box.className).toContain("border-violet-800");
    expect(screen.getByTestId("coaching-box-text").className).toContain(
      "text-violet-200",
    );
    expect(box.outerHTML).not.toContain("amber");
  });

  // AC5: fetch errors are silent — no crash, no box, no notice.
  it("fetch errors are silent — never a crash", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("boom"));
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(screen.queryByTestId("coaching-box")).toBeNull();
  });

  // ---------------------------------------------------------------
  // #454 S3: the second voice — the method-book tip.
  // ---------------------------------------------------------------

  // #454 AC5: with history silent, the tip fills the box — guidance plus a
  // VISIBLE attribution line in the muted violet (attribution is
  // non-negotiable). Fails if the tip stops rendering, the attribution
  // line disappears, or it loses the muted class.
  it("the method-book tip fills the box when history is silent — attribution visible", async () => {
    routeCoachingFetches([], schlossberg);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(mockInvoke).toHaveBeenCalledWith("method_book_tip", undefined);
    expect(screen.getByTestId("coaching-box-text").textContent).toContain(
      "Schlossberg's Daily Drills",
    );
    const attribution = screen.getByTestId("coaching-box-attribution");
    expect(attribution.textContent).toContain(
      "Max Schlossberg, Daily Drills and Technical Studies",
    );
    expect(attribution.className).toContain("text-violet-400/70");
    expect(screen.getAllByTestId("coaching-box")).toHaveLength(1);
  });

  // #454 AC6: history outranks the tip — with both voices present the box
  // shows the history suggestion, no attribution line, still ONE box.
  // Fails if the tip wins the slot or both render at once.
  it("a history suggestion outranks the book tip", async () => {
    routeCoachingFetches([trend], schlossberg);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(screen.getAllByTestId("coaching-box")).toHaveLength(1);
    expect(screen.getByTestId("coaching-box-text").textContent).toContain(
      "Eb major",
    );
    expect(screen.queryByText(/Schlossberg/)).toBeNull();
    expect(screen.queryByTestId("coaching-box-attribution")).toBeNull();
  });

  // #454 AC7: one dismissal quiets BOTH voices for the session — later
  // fetches with a fresh tip AND fresh history stay quiet. Fails if the
  // tip voice survives a dismissal.
  it("dismissal quiets both voices for the session", async () => {
    routeCoachingFetches([], schlossberg);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    fireEvent.click(screen.getByTestId("coaching-box-dismiss"));
    expect(screen.queryByTestId("coaching-box")).toBeNull();
    routeCoachingFetches([trend], schlossberg);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    expect(screen.queryByTestId("coaching-box")).toBeNull();
  });

  // #454 AC7 (rule 0 for the tip voice): an empty tip fetch never clears a
  // shown tip; the calm palette pin holds for the tip rendering too.
  it("the tip holds through an empty fetch — rule 0, calm palette", async () => {
    routeCoachingFetches([], schlossberg);
    render(<CoachingBox />);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    routeCoachingFetches([], null);
    await act(() => usePracticeStore.getState().refreshCoachingSuggestion());
    const box = screen.getByTestId("coaching-box");
    expect(screen.getByTestId("coaching-box-text").textContent).toContain(
      "Schlossberg's Daily Drills",
    );
    expect(box.className).toContain("bg-violet-950/40");
    expect(box.outerHTML).not.toContain("amber");
  });
});
