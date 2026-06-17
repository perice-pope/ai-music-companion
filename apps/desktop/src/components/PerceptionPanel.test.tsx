import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import PerceptionPanel from "./PerceptionPanel";
import { usePracticeStore } from "../stores/practiceStore";
import type { PerceptionSnapshot } from "../types/brain";

const LOCKED_G: PerceptionSnapshot = {
  tempo_bpm: 92.4,
  swing_ratio: null,
  locked: true,
  key: {
    tonic: 7,
    mode: "major",
    name: "G major",
    confidence: 0.8,
    alternative: "E minor",
  },
};

describe("PerceptionPanel", () => {
  beforeEach(() => {
    usePracticeStore.setState({ status: "listening", perception: null });
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
});
