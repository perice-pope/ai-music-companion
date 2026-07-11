import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import PerceptionPanel from "./PerceptionPanel";
import { usePracticeStore } from "../stores/practiceStore";
import type { PerceptionSnapshot } from "../types/brain";
import { colorForPitchClass } from "../lib/rvColors";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const LOCKED_G: PerceptionSnapshot = {
  tempo_bpm: 92.4,
  swing_ratio: null,
  locked: true,
  key: {
    tonic: 7,
    mode: "major",
    name: "G major",
    confidence: 0.8,
    alternative: { tonic: 4, minor: true, name: "E minor" },
  },
};

describe("PerceptionPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue(undefined);
    usePracticeStore.setState({
      status: "listening",
      perception: null,
      keyPinned: false,
    });
  });

  it("renders nothing when no session is listening", () => {
    usePracticeStore.setState({ status: "idle", perception: LOCKED_G });
    const { container } = render(<PerceptionPanel />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows a listening state before anything is heard", () => {
    usePracticeStore.setState({ status: "listening", perception: null });
    render(<PerceptionPanel />);
    screen.getByTestId("perception-listening");
    // No tempo/key surfaced yet.
    expect(screen.queryByTestId("perception-tempo")).toBeNull();
    expect(screen.queryByTestId("perception-key")).toBeNull();
  });

  it("shows the locked tempo and the key with its honest alternative", () => {
    usePracticeStore.setState({ status: "listening", perception: LOCKED_G });
    render(<PerceptionPanel />);
    expect(screen.getByTestId("perception-tempo")).toHaveTextContent("92 BPM");
    const key = screen.getByTestId("perception-key");
    expect(key).toHaveTextContent("G major");
    expect(key).toHaveTextContent("or E minor?");
    // "Listening…" should be gone once we hear something.
    expect(screen.queryByTestId("perception-listening")).toBeNull();
  });

  it("marks the tempo as still-settling when the pulse isn't locked", () => {
    usePracticeStore.setState({
      status: "listening",
      perception: { ...LOCKED_G, locked: false },
    });
    render(<PerceptionPanel />);
    expect(screen.getByTestId("perception-tempo")).toHaveTextContent(
      "finding the pulse",
    );
  });

  it("shows the key tentatively ('maybe') when confidence is low, but still offers the alternative", () => {
    usePracticeStore.setState({
      status: "listening",
      perception: { ...LOCKED_G, key: { ...LOCKED_G.key!, confidence: 0.35 } },
    });
    render(<PerceptionPanel />);
    const key = screen.getByTestId("perception-key");
    expect(key).toHaveTextContent("maybe G major");
    // The honest alternative is shown even (especially) when unsure.
    expect(key).toHaveTextContent("or E minor?");
  });

  it("asserts the key (no 'maybe') when confidence is high", () => {
    usePracticeStore.setState({
      status: "listening",
      perception: { ...LOCKED_G, key: { ...LOCKED_G.key!, confidence: 0.85 } },
    });
    render(<PerceptionPanel />);
    const key = screen.getByTestId("perception-key");
    expect(key).toHaveTextContent("G major");
    expect(key).not.toHaveTextContent("maybe");
  });

  it("always shows the Bluetooth/output speakers tip during a session", () => {
    usePracticeStore.setState({ status: "listening", perception: LOCKED_G });
    render(<PerceptionPanel />);
    expect(screen.getByTestId("perception-output-tip")).toHaveTextContent(
      /Bluetooth/i,
    );
  });

  it("pins the band to the alternative key when the alternative is clicked", () => {
    usePracticeStore.setState({ status: "listening", perception: LOCKED_G });
    render(<PerceptionPanel />);
    fireEvent.click(screen.getByTestId("key-use-alternative"));
    // E minor → tonic 4, minor true.
    expect(mockInvoke).toHaveBeenCalledWith("set_accompaniment_key", {
      tonic: 4,
      minor: true,
    });
    expect(usePracticeStore.getState().keyPinned).toBe(true);
  });

  it("locks the currently-shown key when the lock button is clicked", () => {
    usePracticeStore.setState({ status: "listening", perception: LOCKED_G });
    render(<PerceptionPanel />);
    fireEvent.click(screen.getByTestId("key-lock"));
    // "Lock" pins the concrete current key (G major → tonic 7, not minor).
    expect(mockInvoke).toHaveBeenCalledWith("set_accompaniment_key", {
      tonic: 7,
      minor: false,
    });
    expect(usePracticeStore.getState().keyPinned).toBe(true);
    expect(usePracticeStore.getState().pinnedKey?.name).toBe("G major");
  });

  it("when pinned, shows the PINNED key (not the auto-read), hides the controls, and 'auto' clears", () => {
    // Auto-read says G major, but the user pinned E minor — the panel must show
    // E minor with the lock, not the contradicting auto-read (the must-fix).
    usePracticeStore.setState({
      status: "listening",
      perception: LOCKED_G, // key.name = "G major"
      keyPinned: true,
      pinnedKey: { tonic: 4, minor: true, name: "E minor" },
    });
    render(<PerceptionPanel />);
    const key = screen.getByTestId("perception-key");
    expect(key).toHaveTextContent("E minor");
    expect(key).not.toHaveTextContent("G major");
    // No switch/lock affordance while pinned.
    expect(screen.queryByTestId("key-use-alternative")).toBeNull();
    expect(screen.queryByTestId("key-lock")).toBeNull();
    // The 'auto' control resumes auto-detect.
    fireEvent.click(screen.getByTestId("key-auto"));
    expect(mockInvoke).toHaveBeenCalledWith("clear_accompaniment_key");
    expect(usePracticeStore.getState().keyPinned).toBe(false);
  });

  // #349 T1c AC: the live chord label renders with the RV root colour dot.
  it("shows the heard chord with its RV root colour", () => {
    usePracticeStore.setState({
      status: "listening",
      perception: {
        ...LOCKED_G,
        chord: { root_pc: 0, label: "Cmaj7", bass_pc: null, confidence: 0.82 },
      },
    });
    render(<PerceptionPanel />);
    const chord = screen.getByTestId("perception-chord");
    expect(chord).toHaveTextContent("Cmaj7");
    // The dot must wear EXACTLY the RV colour of the root pitch class —
    // not just any colour (a hardwired palette entry must fail here).
    const dot = chord.querySelector("span[aria-hidden]");
    expect(dot).not.toBeNull();
    const probe = document.createElement("span");
    probe.style.backgroundColor = colorForPitchClass(0);
    expect((dot as HTMLElement).style.backgroundColor).toBe(
      probe.style.backgroundColor,
    );
    expect(screen.queryByTestId("perception-polyphony")).toBeNull();
  });

  // #349 §5.3 honesty: polyphony without a confident name says so — it
  // never invents a label.
  it("says 'hearing several notes…' when polyphony has no confident name", () => {
    usePracticeStore.setState({
      status: "listening",
      perception: {
        tempo_bpm: null,
        swing_ratio: null,
        locked: false,
        key: null,
        chord: null,
        hearing_polyphony: true,
      },
    });
    render(<PerceptionPanel />);
    expect(screen.getByTestId("perception-polyphony")).toHaveTextContent(
      "hearing several notes",
    );
    expect(screen.queryByTestId("perception-chord")).toBeNull();
    // The honest fallback counts as "hearing something" — no listening state.
    expect(screen.queryByTestId("perception-listening")).toBeNull();
  });

  // An inversion label ("C/E") arrives pre-spelled from the backend and
  // renders verbatim — the frontend must not re-derive or mangle it.
  it("renders slash-chord labels verbatim", () => {
    usePracticeStore.setState({
      status: "listening",
      perception: {
        ...LOCKED_G,
        chord: { root_pc: 0, label: "C7/E", bass_pc: 4, confidence: 0.7 },
      },
    });
    render(<PerceptionPanel />);
    expect(screen.getByTestId("perception-chord")).toHaveTextContent("C7/E");
  });
});
