import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import InstrumentSelector from "./InstrumentSelector";
import { useAudioStore, type InstrumentInfo } from "../stores/audioStore";
import { usePracticeStore } from "../stores/practiceStore";

/**
 * Fixture catalog that mirrors the real `profiles/*.json` shape. The
 * frontend is now data-driven by the backend, so the test supplies the
 * catalog through the mocked `invoke("list_instruments")` call. Add a
 * row here when you add a new profile JSON and the component should
 * render it.
 */
const TEST_INSTRUMENTS: InstrumentInfo[] = [
  {
    name: "Trumpet",
    family: "Brass",
    freqMinHz: 165,
    freqMaxHz: 1047,
    vibratoToleranceCents: 20,
    emoji: "\uD83C\uDFBA",
    polyphonic: false,
  },
  {
    name: "Trombone",
    family: "Brass",
    freqMinHz: 58,
    freqMaxHz: 587,
    vibratoToleranceCents: 20,
    emoji: "\uD83C\uDFB5",
    polyphonic: false,
  },
  {
    name: "French Horn",
    family: "Brass",
    freqMinHz: 87,
    freqMaxHz: 880,
    vibratoToleranceCents: 20,
    emoji: "\uD83D\uDCEF",
    polyphonic: false,
  },
  {
    name: "Violin",
    family: "Strings",
    freqMinHz: 196,
    freqMaxHz: 3136,
    vibratoToleranceCents: 30,
    emoji: "\uD83C\uDFBB",
    polyphonic: false,
  },
  {
    name: "Cello",
    family: "Strings",
    freqMinHz: 65,
    freqMaxHz: 988,
    vibratoToleranceCents: 30,
    emoji: "\uD83C\uDFB6",
    polyphonic: false,
  },
  {
    name: "Flute",
    family: "Woodwind",
    freqMinHz: 262,
    freqMaxHz: 2093,
    vibratoToleranceCents: 20,
    emoji: "\uD83C\uDFB6",
    polyphonic: false,
  },
  {
    name: "Clarinet",
    family: "Woodwind",
    freqMinHz: 147,
    freqMaxHz: 1568,
    vibratoToleranceCents: 20,
    emoji: "\uD83C\uDFB5",
    polyphonic: false,
  },
  {
    name: "Voice",
    family: "Voice",
    freqMinHz: 82,
    freqMaxHz: 1047,
    vibratoToleranceCents: 35,
    emoji: "\uD83C\uDFA4",
    polyphonic: false,
  },
  {
    name: "Piano",
    family: "Keyboard",
    freqMinHz: 28,
    freqMaxHz: 4186,
    vibratoToleranceCents: 10,
    emoji: "\uD83C\uDFB9",
    polyphonic: true,
  },
];

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// The version badge in the picker footer (#384) reads the bundle version
// through the app API; give it a stable value here.
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: () => Promise.resolve("2.29.0"),
}));

// Route `list_instruments` to the fixture and forward everything else
// to per-test `.mockResolvedValueOnce` setups.
function installDefaultInvokeMock() {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "list_instruments") {
      return Promise.resolve(TEST_INSTRUMENTS);
    }
    return Promise.reject(new Error(`no mock configured for invoke("${cmd}")`));
  });
}

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
    installDefaultInvokeMock();
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
      // Reset practiceMode explicitly — the store loads it from
      // localStorage at module-init time, and one test flipping it
      // would otherwise leak into the next.
      practiceMode: "practice",
    });
  });

  it("shows the running app version in the footer (#384)", async () => {
    render(<InstrumentSelector />);
    expect(await screen.findByTestId("app-version")).toHaveTextContent(
      "v2.29.0",
    );
  });

  it("renders all instrument cards from the backend catalog", async () => {
    render(<InstrumentSelector />);

    for (const instrument of TEST_INSTRUMENTS) {
      const testId = `instrument-card-${instrument.name.toLowerCase().replace(/\s+/g, "-")}`;
      expect(await screen.findByTestId(testId)).toBeDefined();
    }
  });

  it("renders correct number of instruments", async () => {
    render(<InstrumentSelector />);

    expect(TEST_INSTRUMENTS.length).toBe(9);
    for (const instrument of TEST_INSTRUMENTS) {
      const testId = `instrument-card-${instrument.name.toLowerCase().replace(/\s+/g, "-")}`;
      expect(await screen.findByTestId(testId)).toBeDefined();
    }
  });

  it("displays family badges", async () => {
    render(<InstrumentSelector />);
    await screen.findByTestId("instrument-card-trumpet");

    expect(screen.getAllByText("Brass").length).toBe(3);
    expect(screen.getAllByText("Strings").length).toBe(2);
    expect(screen.getAllByText("Woodwind").length).toBe(2);
    // Voice appears twice: as instrument name and as family badge (since Voice.family === "Voice")
    expect(screen.getAllByText("Voice").length).toBe(2);
    expect(screen.getAllByText("Keyboard").length).toBe(1);
  });

  it("invokes list_instruments exactly once on mount", async () => {
    render(<InstrumentSelector />);
    await screen.findByTestId("instrument-card-trumpet");

    const catalogCalls = mockInvoke.mock.calls.filter(
      (args) => args[0] === "list_instruments",
    );
    expect(catalogCalls.length).toBe(1);
  });

  it("renders a loading indicator before the catalog resolves", () => {
    // Override so the promise never resolves within the assertion window.
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return new Promise(() => {});
      return Promise.reject(new Error("unexpected"));
    });
    render(<InstrumentSelector />);
    expect(screen.getByTestId("catalog-loading")).toBeDefined();
  });

  it("surfaces a catalog-load error", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") {
        return Promise.reject(new Error("no profiles dir"));
      }
      return Promise.reject(new Error("unexpected"));
    });
    render(<InstrumentSelector />);
    const err = await screen.findByTestId("catalog-error");
    expect(err.textContent).toContain("no profiles dir");
  });

  it("clicking an instrument updates the store", async () => {
    render(<InstrumentSelector />);

    const trumpetCard = await screen.findByTestId("instrument-card-trumpet");
    fireEvent.click(trumpetCard);

    const state = useAudioStore.getState();
    expect(state.selectedInstrument).toBe("Trumpet");
  });

  it("selected instrument is visually highlighted", async () => {
    useAudioStore.setState({ selectedInstrument: "Violin" });
    render(<InstrumentSelector />);

    const violinCard = await screen.findByTestId("instrument-card-violin");
    expect(violinCard.getAttribute("aria-pressed")).toBe("true");

    // Check that unselected card is not pressed
    const trumpetCard = screen.getByTestId("instrument-card-trumpet");
    expect(trumpetCard.getAttribute("aria-pressed")).toBe("false");
  });

  it("shows selected indicator on chosen instrument", async () => {
    useAudioStore.setState({ selectedInstrument: "Piano" });
    render(<InstrumentSelector />);

    expect(await screen.findByTestId("selected-indicator")).toBeDefined();
  });

  it("persists selection to localStorage", async () => {
    render(<InstrumentSelector />);

    const celloCard = await screen.findByTestId("instrument-card-cello");
    fireEvent.click(celloCard);

    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "ai-music-companion:selected-instrument",
      "Cello",
    );
  });

  it("switching instruments updates store and localStorage", async () => {
    render(<InstrumentSelector />);

    // Select trumpet
    fireEvent.click(await screen.findByTestId("instrument-card-trumpet"));
    expect(useAudioStore.getState().selectedInstrument).toBe("Trumpet");

    // Switch to flute
    fireEvent.click(screen.getByTestId("instrument-card-flute"));
    expect(useAudioStore.getState().selectedInstrument).toBe("Flute");
    expect(localStorageMock.setItem).toHaveBeenLastCalledWith(
      "ai-music-companion:selected-instrument",
      "Flute",
    );
  });

  it("displays frequency ranges", async () => {
    render(<InstrumentSelector />);
    await screen.findByTestId("instrument-card-trumpet");

    // Check that trumpet frequency range is displayed
    expect(screen.getByText(/165 .* 1047 Hz/)).toBeDefined();
  });

  it("renders the section heading", () => {
    render(<InstrumentSelector />);
    expect(screen.getByText("Select Your Instrument")).toBeDefined();
  });

  it("Start Practice is disabled until an instrument is selected", async () => {
    render(<InstrumentSelector />);
    const btn = screen.getByTestId(
      "start-practice-button",
    ) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);

    fireEvent.click(await screen.findByTestId("instrument-card-trumpet"));
    expect(
      (screen.getByTestId("start-practice-button") as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("Start Practice invokes start_practice_session and navigates to session", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      if (cmd === "start_practice_session")
        return Promise.resolve("new-session-id");
      return Promise.reject(new Error(`unexpected cmd ${cmd}`));
    });
    render(<InstrumentSelector />);
    fireEvent.click(await screen.findByTestId("instrument-card-violin"));
    fireEvent.click(screen.getByTestId("start-practice-button"));

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("start_practice_session", {
        instrument: "Violin",
        practiceMode: "practice",
        // Off by default (offline-first): narration is opt-in.
        coachingEnabled: false,
        scoreId: null,
      });
    });
    await vi.waitFor(() => {
      expect(usePracticeStore.getState().status).toBe("listening");
    });
    expect(usePracticeStore.getState().screen).toBe("session");
    expect(usePracticeStore.getState().sessionId).toBe("new-session-id");
  });

  it("renders the practice mode selector with 'Practice' active by default", async () => {
    render(<InstrumentSelector />);
    const selector = await screen.findByTestId("practice-mode-selector");
    expect(selector).toBeDefined();

    const practiceBtn = screen.getByTestId("practice-mode-practice");
    expect(practiceBtn.getAttribute("aria-checked")).toBe("true");

    const warmupBtn = screen.getByTestId("practice-mode-warmup");
    expect(warmupBtn.getAttribute("aria-checked")).toBe("false");
  });

  it("clicking a mode updates the store and the description", async () => {
    render(<InstrumentSelector />);
    fireEvent.click(await screen.findByTestId("practice-mode-warmup"));

    expect(usePracticeStore.getState().practiceMode).toBe("warmup");
    expect(
      screen.getByTestId("practice-mode-warmup").getAttribute("aria-checked"),
    ).toBe("true");
    // The description copy swaps to the Warm-up blurb.
    const desc = screen.getByTestId("practice-mode-description");
    expect(desc.textContent).toMatch(/silent/i);
  });

  it("Start Practice passes the selected mode through to invoke", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      if (cmd === "start_practice_session")
        return Promise.resolve("sid-run-through");
      return Promise.reject(new Error(`unexpected cmd ${cmd}`));
    });
    render(<InstrumentSelector />);
    fireEvent.click(await screen.findByTestId("instrument-card-trumpet"));
    fireEvent.click(screen.getByTestId("practice-mode-run_through"));
    fireEvent.click(screen.getByTestId("start-practice-button"));

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("start_practice_session", {
        instrument: "Trumpet",
        practiceMode: "run_through",
        // Off by default (offline-first): narration is opt-in.
        coachingEnabled: false,
        scoreId: null,
      });
    });
  });

  it("surfaces a backend error if start fails", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      if (cmd === "start_practice_session")
        return Promise.reject(new Error("mic busy"));
      return Promise.reject(new Error(`unexpected cmd ${cmd}`));
    });
    render(<InstrumentSelector />);
    fireEvent.click(await screen.findByTestId("instrument-card-piano"));
    fireEvent.click(screen.getByTestId("start-practice-button"));

    const err = await screen.findByTestId("start-error");
    expect(err.textContent).toContain("mic busy");
    // And we stayed on the selector screen.
    expect(usePracticeStore.getState().screen).toBe("selector");
    expect(usePracticeStore.getState().status).toBe("idle");
  });
});
