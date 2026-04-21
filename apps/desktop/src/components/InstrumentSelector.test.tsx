import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import InstrumentSelector, { INSTRUMENTS } from "./InstrumentSelector";
import { useAudioStore } from "../stores/audioStore";
import { usePracticeStore } from "../stores/practiceStore";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Mock localStorage
const localStorageMock = (() => {
  let store: Record<string, string> = {};
  return {
    getItem: vi.fn((key: string) => store[key] ?? null),
    setItem: vi.fn((key: string, value: string) => {
      store[key] = value;
    }),
    removeItem: vi.fn((key: string) => {
      delete store[key];
    }),
    clear: vi.fn(() => {
      store = {};
    }),
    get length() {
      return Object.keys(store).length;
    },
    key: vi.fn((_index: number) => null),
  };
})();

Object.defineProperty(window, "localStorage", { value: localStorageMock });

describe("InstrumentSelector", () => {
  beforeEach(() => {
    localStorageMock.clear();
    vi.clearAllMocks();
    useAudioStore.setState({
      latestEvent: null,
      currentNote: null,
      isListening: false,
      selectedInstrument: null,
    });
    usePracticeStore.setState({
      screen: "selector",
      status: "idle",
      sessionId: null,
      instrumentName: null,
      segmentId: null,
      startedAtMs: null,
      elapsedSecs: 0,
      phrases: [],
      tipQueue: [],
      recap: null,
      recapError: null,
    });
  });

  it("renders all instrument cards", () => {
    render(<InstrumentSelector />);

    for (const instrument of INSTRUMENTS) {
      const testId = `instrument-card-${instrument.name.toLowerCase().replace(/\s+/g, "-")}`;
      expect(screen.getByTestId(testId)).toBeDefined();
    }
  });

  it("renders correct number of instruments", () => {
    render(<InstrumentSelector />);

    // All 9 instruments should be rendered
    expect(INSTRUMENTS.length).toBe(9);
    for (const instrument of INSTRUMENTS) {
      expect(screen.getByText(instrument.name)).toBeDefined();
    }
  });

  it("displays family badges", () => {
    render(<InstrumentSelector />);

    expect(screen.getAllByText("Brass").length).toBe(3);
    expect(screen.getAllByText("Strings").length).toBe(2);
    expect(screen.getAllByText("Woodwind").length).toBe(2);
    expect(screen.getAllByText("Voice").length).toBe(1);
    expect(screen.getAllByText("Keyboard").length).toBe(1);
  });

  it("clicking an instrument updates the store", () => {
    render(<InstrumentSelector />);

    const trumpetCard = screen.getByTestId("instrument-card-trumpet");
    fireEvent.click(trumpetCard);

    const state = useAudioStore.getState();
    expect(state.selectedInstrument).toBe("Trumpet");
  });

  it("selected instrument is visually highlighted", () => {
    useAudioStore.setState({ selectedInstrument: "Violin" });
    render(<InstrumentSelector />);

    const violinCard = screen.getByTestId("instrument-card-violin");
    expect(violinCard.getAttribute("aria-pressed")).toBe("true");

    // Check that unselected card is not pressed
    const trumpetCard = screen.getByTestId("instrument-card-trumpet");
    expect(trumpetCard.getAttribute("aria-pressed")).toBe("false");
  });

  it("shows selected indicator on chosen instrument", () => {
    useAudioStore.setState({ selectedInstrument: "Piano" });
    render(<InstrumentSelector />);

    expect(screen.getByTestId("selected-indicator")).toBeDefined();
  });

  it("persists selection to localStorage", () => {
    render(<InstrumentSelector />);

    const celloCard = screen.getByTestId("instrument-card-cello");
    fireEvent.click(celloCard);

    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:selected-instrument",
      "Cello",
    );
  });

  it("switching instruments updates store and localStorage", () => {
    render(<InstrumentSelector />);

    // Select trumpet
    fireEvent.click(screen.getByTestId("instrument-card-trumpet"));
    expect(useAudioStore.getState().selectedInstrument).toBe("Trumpet");

    // Switch to flute
    fireEvent.click(screen.getByTestId("instrument-card-flute"));
    expect(useAudioStore.getState().selectedInstrument).toBe("Flute");
    expect(localStorageMock.setItem).toHaveBeenLastCalledWith(
      "ai-music-companion:selected-instrument",
      "Flute",
    );
  });

  it("displays frequency ranges", () => {
    render(<InstrumentSelector />);

    // Check that trumpet frequency range is displayed
    expect(screen.getByText(/165 .* 1047 Hz/)).toBeDefined();
  });

  it("renders the section heading", () => {
    render(<InstrumentSelector />);
    expect(screen.getByText("Select Your Instrument")).toBeDefined();
  });

  it("Start Practice is disabled until an instrument is selected", () => {
    render(<InstrumentSelector />);
    const btn = screen.getByTestId("start-practice-button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);

    fireEvent.click(screen.getByTestId("instrument-card-trumpet"));
    expect(
      (screen.getByTestId("start-practice-button") as HTMLButtonElement).disabled,
    ).toBe(false);
  });

  it("Start Practice invokes start_practice_session and navigates to session", async () => {
    mockInvoke.mockResolvedValueOnce("new-session-id");
    render(<InstrumentSelector />);
    fireEvent.click(screen.getByTestId("instrument-card-violin"));
    fireEvent.click(screen.getByTestId("start-practice-button"));

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("start_practice_session", {
        instrument: "Violin",
        coachingEnabled: true,
      });
    });
    await vi.waitFor(() => {
      expect(usePracticeStore.getState().status).toBe("listening");
    });
    expect(usePracticeStore.getState().screen).toBe("session");
    expect(usePracticeStore.getState().sessionId).toBe("new-session-id");
  });

  it("surfaces a backend error if start fails", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("mic busy"));
    render(<InstrumentSelector />);
    fireEvent.click(screen.getByTestId("instrument-card-piano"));
    fireEvent.click(screen.getByTestId("start-practice-button"));

    const err = await screen.findByTestId("start-error");
    expect(err.textContent).toContain("mic busy");
    // And we stayed on the selector screen.
    expect(usePracticeStore.getState().screen).toBe("selector");
    expect(usePracticeStore.getState().status).toBe("idle");
  });
});
