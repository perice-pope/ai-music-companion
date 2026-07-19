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

beforeEach(() => {
  mockInvoke.mockReset();
  usePracticeStore.setState({ pieceMatch: null, dismissedPieceIds: [] });
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
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-2",
      title: "Gymnopédie No. 1",
      coherent_hits: 8,
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
    });
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-1",
      title: "Für Elise",
      coherent_hits: 9,
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
    });
    render(<PieceMatchChip />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.queryByTestId("piece-match-chip")).toBeNull();
    usePracticeStore.setState({ activeScore: null });
  });
});
