import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
} from "@testing-library/react";

/**
 * #214 S1b — the chip: appears on a gated match, HOLDS (rule 0),
 * replaces in place, and a dismissal quiets that score for the session.
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import PieceMatchChip from "./PieceMatchChip";
import { usePracticeStore } from "../stores/practiceStore";
import { useAudioStore } from "../stores/audioStore";

beforeEach(() => {
  mockInvoke.mockReset();
  usePracticeStore.setState({
    pieceMatch: null,
    dismissedPieceIds: [],
    // S2 tests stage the matched score and flip session status — keep
    // tests isolated (a leaked activeScore trips the echo guard; a
    // leaked status flips the teardown path).
    activeScore: null,
    activeScoreXml: null,
    screen: "selector" as never,
    status: "idle" as never,
  });
  useAudioStore.setState({ isListening: false });
});

describe("PieceMatchChip (#214 S1b)", () => {
  it("renders nothing until identification asserts", () => {
    render(<PieceMatchChip />);
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
  });

  it("a gated match surfaces and HOLDS through misses — rule 0", async () => {
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-1",
      title: "Für Elise",
      coherent_hits: 9,
      confirmed: false,
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.getByTestId("piece-match-title").textContent).toContain(
      "Für Elise",
    );
    // The next phrase returns None — the chip must NOT clear.
    mockInvoke.mockResolvedValueOnce(null);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.getByTestId("piece-match-chip")).toBeInTheDocument();
  });

  it("a newer match replaces in place; dismissal quiets that score", async () => {
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-1",
      title: "Für Elise",
      coherent_hits: 9,
      confirmed: false,
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-2",
      title: "Gymnopédie No. 1",
      coherent_hits: 8,
      confirmed: false,
    });
    await act(() => usePracticeStore.getState().requestPieceMatch());
    await waitFor(() =>
      expect(screen.getByTestId("piece-match-title").textContent).toContain(
        "Gymnopédie",
      ),
    );
    fireEvent.click(screen.getByTestId("piece-match-dismiss"));
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
    // The dismissed score returning stays QUIET; a different one shows.
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-2",
      title: "Gymnopédie No. 1",
      coherent_hits: 8,
      confirmed: false,
    });
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-1",
      title: "Für Elise",
      coherent_hits: 9,
      confirmed: false,
    });
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.getByTestId("piece-match-title").textContent).toContain(
      "Für Elise",
    );
  });

  it("identification errors are silent — never a crash, never a notice", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("boom"));
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
  });

  it("the score you have OPEN never echoes as a match — MF3", async () => {
    usePracticeStore.setState({
      activeScore: { id: "id-open" } as never,
    });
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-open",
      title: "The Open Score",
      coherent_hits: 9,
      confirmed: false,
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
    usePracticeStore.setState({ activeScore: null });
  });

  // #214 S2 (S1b carried note): the echo guard suppresses only the score
  // you have OPEN — a DIFFERENT score identifying while one is open must
  // still surface. Fails if the guard over-suppresses to any-score-open.
  it("a DIFFERENT score than the open one still surfaces", async () => {
    usePracticeStore.setState({
      activeScore: { id: "id-open" } as never,
    });
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-other",
      title: "The Other Piece",
      coherent_hits: 9,
      confirmed: false,
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.getByTestId("piece-match-title").textContent).toContain(
      "The Other Piece",
    );
    usePracticeStore.setState({ activeScore: null });
  });

  // #214 S2: one tap opens the matched score — staged as the active
  // score, landing on the selector, match consumed. Fails if the tap
  // navigates without the score or leaves a stale match behind.
  it("Open score stages the match and lands on the selector", async () => {
    usePracticeStore.setState({ screen: "session" as never });
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-1",
      title: "Für Elise",
      coherent_hits: 9,
      confirmed: false,
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    mockInvoke.mockResolvedValueOnce({
      entry: { id: "id-1", title: "Für Elise" },
      music_xml: "<score-partwise/>",
    });
    fireEvent.click(screen.getByTestId("piece-match-open"));
    await waitFor(() =>
      expect(usePracticeStore.getState().screen).toBe("selector"),
    );
    expect(mockInvoke).toHaveBeenLastCalledWith("get_score", { id: "id-1" });
    const s = usePracticeStore.getState();
    expect(s.activeScore).toMatchObject({ id: "id-1" });
    expect(s.activeScoreXml).toBe("<score-partwise/>");
    expect(s.pieceMatch).toBeNull();
  });

  // #214 S2: a score that can't load (deleted mid-session) gets a calm
  // notice, goes quiet for the session, and the player STAYS PUT — never
  // yanked to the selector for nothing, never a door that doesn't open.
  it("a score that can't load: calm notice, quieted, no navigation", async () => {
    usePracticeStore.setState({ screen: "session" as never });
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-1",
      title: "Für Elise",
      coherent_hits: 9,
      confirmed: false,
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    mockInvoke.mockRejectedValueOnce("score not found: id-1");
    fireEvent.click(screen.getByTestId("piece-match-open"));
    await waitFor(() =>
      expect(screen.getByTestId("piece-match-notice")).toBeInTheDocument(),
    );
    const s = usePracticeStore.getState();
    expect(s.screen).toBe("session");
    expect(s.pieceMatch).toBeNull();
    expect(s.dismissedPieceIds).toContain("id-1");
    // The notice is dismissible, and the chip stays gone (quieted).
    fireEvent.click(screen.getByTestId("piece-match-notice-dismiss"));
    expect(screen.queryByTestId("piece-match-notice")).toBeNull();
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
  });

  // #214 S2 review MF1: the chip lives on LIVE session screens — Open
  // score mid-session must END the backend session (mic pipeline, band,
  // recorder), or the selector inherits a hot mic and the next start
  // soft-locks on AlreadyActive. Fails if returnToSelector goes alone.
  it("Open score mid-session ends the backend session first", async () => {
    usePracticeStore.setState({
      screen: "session" as never,
      status: "listening" as never,
      // Round-2 mutant j: seed NON-defaults so the endSession-tail reset
      // assertions below have teeth (defaults would pass vacuously).
      dismissedPieceIds: ["id-x"],
      pocketMode: "follow",
    });
    useAudioStore.setState({ isListening: true });
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "check_piece_match") {
        return Promise.resolve({
          score_id: "id-1",
          title: "Für Elise",
          coherent_hits: 9,
          confirmed: false,
        });
      }
      if (cmd === "get_score") {
        return Promise.resolve({
          entry: { id: "id-1", title: "Für Elise" },
          music_xml: "<score-partwise/>",
        });
      }
      if (cmd === "end_practice_session") {
        return Promise.resolve({});
      }
      return Promise.resolve(null);
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    fireEvent.click(screen.getByTestId("piece-match-open"));
    await waitFor(() =>
      expect(usePracticeStore.getState().screen).toBe("selector"),
    );
    const calls = mockInvoke.mock.calls.map((c) => c[0]);
    expect(calls).toContain("end_practice_session");
    const s = usePracticeStore.getState();
    expect(s.status).toBe("idle");
    expect(useAudioStore.getState().isListening).toBe(false);
    expect(s.activeScore).toMatchObject({ id: "id-1" });
    // Session-scoped state resets like a real session end.
    expect(s.dismissedPieceIds).toEqual([]);
    expect(s.pocketMode).toBe("anchor");
  });

  // The idle-path counterpart: no live session, no end call — the tap
  // must not fire teardown at a backend with nothing to tear down.
  it("Open score outside a live session skips the session end", async () => {
    usePracticeStore.setState({
      screen: "session" as never,
      status: "idle" as never,
    });
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "check_piece_match") {
        return Promise.resolve({
          score_id: "id-1",
          title: "Für Elise",
          coherent_hits: 9,
          confirmed: false,
        });
      }
      if (cmd === "get_score") {
        return Promise.resolve({
          entry: { id: "id-1", title: "Für Elise" },
          music_xml: "<score-partwise/>",
        });
      }
      return Promise.resolve(null);
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    fireEvent.click(screen.getByTestId("piece-match-open"));
    await waitFor(() =>
      expect(usePracticeStore.getState().screen).toBe("selector"),
    );
    expect(mockInvoke.mock.calls.map((c) => c[0])).not.toContain(
      "end_practice_session",
    );
  });

  // S2 review note 3: a fresh match must outrank a lingering failure
  // notice — the surface never suppresses live identification.
  it("a new match replaces a lingering notice", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "check_piece_match") {
        return Promise.resolve({
          score_id: "id-2",
          title: "Gymnopédie No. 1",
          coherent_hits: 8,
          confirmed: false,
        });
      }
      if (cmd === "get_score") {
        return Promise.reject("score not found");
      }
      return Promise.resolve(null);
    });
    usePracticeStore.setState({
      pieceMatch: { scoreId: "id-1", title: "Für Elise", confirmed: false },
    });
    render(<PieceMatchChip />);
    fireEvent.click(screen.getByTestId("piece-match-open"));
    await waitFor(() =>
      expect(screen.getByTestId("piece-match-notice")).toBeInTheDocument(),
    );
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.queryByTestId("piece-match-notice")).toBeNull();
    expect(screen.getByTestId("piece-match-title").textContent).toContain(
      "Gymnopédie",
    );
  });
});
