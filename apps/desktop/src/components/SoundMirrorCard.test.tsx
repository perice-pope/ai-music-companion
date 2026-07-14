import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import SoundMirrorCard from "./SoundMirrorCard";
import type { SoundMirrorDto, SoundProfileDto } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

function profile(overrides?: Partial<SoundProfileDto>): SoundProfileDto {
  return {
    sessions_counted: 6,
    mode_lean: "minor",
    feel: "swung",
    comparison: null,
    confidence: 0.7,
    derived_at: 100,
    ...overrides,
  };
}

describe("SoundMirrorCard (#258)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // #258 AC (empty state): below the threshold the mirror invites — counts
  // toward K, guesses NOTHING.
  it("empty state counts toward the threshold without guessing", async () => {
    mockInvoke.mockResolvedValueOnce({
      profile: null,
      sessions_seen: 2,
    } satisfies SoundMirrorDto);
    render(<SoundMirrorCard />);
    await waitFor(() =>
      expect(screen.getByTestId("mirror-empty")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("mirror-empty")).toHaveTextContent(
      "2 of 5 sessions",
    );
    expect(screen.queryByTestId("mirror-axes")).not.toBeInTheDocument();
  });

  // #258 AC: resolved axes read as one human sentence; a grounded comparison
  // gets its own line.
  it("renders the axes sentence and the grounded comparison", async () => {
    mockInvoke.mockResolvedValueOnce({
      profile: profile({
        comparison: { label: "shades of Santana — “Oye Como Va”", source: "grounded" },
      }),
      sessions_seen: 6,
    } satisfies SoundMirrorDto);
    render(<SoundMirrorCard />);
    await waitFor(() =>
      expect(screen.getByTestId("mirror-axes")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("mirror-axes")).toHaveTextContent(
      "You lean into darker, minor colors with a swung, laid-back feel.",
    );
    expect(screen.getByTestId("mirror-comparison")).toHaveTextContent(
      "shades of Santana",
    );
  });

  // #258 AC: profile present but no comparison → the quiet "still listening…"
  // line, never a fabricated name.
  it("shows still listening when no comparison is grounded", async () => {
    mockInvoke.mockResolvedValueOnce({
      profile: profile(),
      sessions_seen: 6,
    } satisfies SoundMirrorDto);
    render(<SoundMirrorCard />);
    await waitFor(() =>
      expect(screen.getByTestId("mirror-listening")).toBeInTheDocument(),
    );
    expect(screen.queryByTestId("mirror-comparison")).not.toBeInTheDocument();
  });

  // Axes all absent (measured sessions carried no key/groove): fall back to
  // the invite rather than an empty sentence.
  it("falls back to the invite when no axis resolved", async () => {
    mockInvoke.mockResolvedValueOnce({
      profile: profile({ mode_lean: null, feel: null }),
      sessions_seen: 6,
    } satisfies SoundMirrorDto);
    render(<SoundMirrorCard />);
    await waitFor(() =>
      expect(screen.getByTestId("mirror-empty")).toBeInTheDocument(),
    );
  });

  it("renders nothing when the fetch fails", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("no backend"));
    const { container } = render(<SoundMirrorCard />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(container.querySelector("[data-testid=sound-mirror]")).toBeNull();
  });
});
