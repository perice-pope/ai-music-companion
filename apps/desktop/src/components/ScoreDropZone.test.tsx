import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  act,
} from "@testing-library/react";
import ScoreDropZone from "./ScoreDropZone";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Capture `import-progress` listeners so tests can simulate backend progress.
const { progressHandlers } = vi.hoisted(() => ({
  progressHandlers: [] as Array<(e: { payload: unknown }) => void>,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void) => {
    if (event === "import-progress") progressHandlers.push(handler);
    return Promise.resolve(() => {});
  },
}));

const ENTRY = {
  id: "00000000-0000-0000-0000-000000000001",
  title: "scales",
  composer: null,
  source_filename: "scales.mid",
  added_at: "2026-05-30T00:00:00Z",
  last_practiced_at: null,
  part_index: 0,
  duration_measures: 1,
};

const AUDIO_RESULT = {
  entry: { ...ENTRY, title: "recording", source_filename: "recording.wav" },
  note_count: 5,
  mean_confidence: 0.82,
  polyphony: 0.0,
  polyphonic: false,
  low_confidence: false,
  uncertain_count: 0,
  verdict: "mono" as const,
};

function installInvokeMock() {
  mockInvoke.mockImplementation((cmd: string) => {
    // #336 diagnostics: fire-and-forget log lines; accept them.
    if (cmd === "frontend_breadcrumb") return Promise.resolve();
    if (cmd === "import_midi_file") return Promise.resolve(ENTRY);
    if (cmd === "import_audio_file") return Promise.resolve(AUDIO_RESULT);
    if (cmd === "import_musicxml_file") return Promise.resolve(ENTRY);
    // Default: a single-part recognized PDF, so the picker is skipped.
    if (cmd === "recognize_pdf_score")
      return Promise.resolve({
        music_xml: "<score-partwise/>",
        parts: ["Piano"],
        from_scan: true,
        low_content: false,
      });
    // Default: single-part score, so the picker is skipped.
    if (cmd === "list_score_parts") return Promise.resolve(["Flute"]);
    // Default: single playable MIDI track, so the picker is skipped.
    if (cmd === "list_midi_parts")
      return Promise.resolve([
        { track_index: 0, name: "Track 1", note_count: 4 },
      ]);
    if (cmd === "get_score")
      return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
    return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
  });
}

// jsdom's File doesn't implement arrayBuffer(); the real Tauri webview
// (Chromium/WebKit) does. Build a File with a working arrayBuffer for tests.
function fileWithBytes(name: string, bytes: number[]): File {
  const data = new Uint8Array(bytes);
  const file = new File([data], name);
  Object.defineProperty(file, "arrayBuffer", {
    value: async () => data.buffer,
  });
  return file;
}

// The #336 first-frame probe rides requestAnimationFrame and may sample a
// second frame; a test that ends mid-probe would otherwise leak the probe's
// breadcrumb into the NEXT test's mock history. Wait two real frames so any
// pending probe finishes before the next test's beforeEach clears the mocks.
afterEach(async () => {
  const nextFrame = () =>
    new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  await nextFrame();
  await nextFrame();
});

function fileInput(): HTMLInputElement {
  // The drop zone's <input type="file"> is hidden but present in the DOM.
  const input = document.querySelector('input[type="file"]');
  if (!input) throw new Error("file input not found");
  return input as HTMLInputElement;
}

describe("ScoreDropZone", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    progressHandlers.length = 0;
    installInvokeMock();
  });

  it("routes a .mid file to import_midi_file with its bytes", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("scales.mid", [0x4d, 0x54, 0x68, 0x64]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_midi_file", {
        sourceFilename: "scales.mid",
        bytes: [0x4d, 0x54, 0x68, 0x64],
        trackIndex: null,
      }),
    );
    await screen.findByText(/Imported "scales"/);
  });

  it("routes a .midi file to import_midi_file too", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("piece.midi", [1, 2, 3]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "import_midi_file",
        expect.objectContaining({ sourceFilename: "piece.midi" }),
      ),
    );
  });

  // #337 S1: a band MIDI (several playable tracks) asks WHICH part to
  // practice — same picker as multi-part MusicXML — and imports the chosen
  // ORIGINAL track index, not the list position.
  it("asks which track to practice for a multi-track MIDI, then imports the choice", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_midi_parts")
        return Promise.resolve([
          { track_index: 1, name: "Trumpet", note_count: 4 },
          { track_index: 3, name: "Bass", note_count: 2 },
        ]);
      if (cmd === "import_midi_file") return Promise.resolve(ENTRY);
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("band.mid", [9, 9]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    // The picker names the playable tracks with note counts.
    await screen.findByText("Bass (2 notes)");
    expect(
      mockInvoke.mock.calls.filter((c) => c[0] === "import_midi_file"),
    ).toHaveLength(0);

    fireEvent.click(screen.getByText("Bass (2 notes)"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_midi_file", {
        sourceFilename: "band.mid",
        bytes: [9, 9],
        trackIndex: 3,
      }),
    );
    await screen.findByText(/Imported "scales"/);
  });

  // #337 S1 AC2: the format hint tells the truth about reliability tiers —
  // exact copy (escaped), both tiers, both beta formats named.
  it("labels the stable and beta format tiers", () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    expect(
      screen.getByText(/Reliable: \.musicxml, \.xml, \.mid, \.midi scores/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /Beta, still rough:\s*\.pdf scans, \.wav\/\.mp3 recordings/,
      ),
    ).toBeInTheDocument();
  });

  // #337 S1 AC2 (second clause): a .wav import shows its beta label AT
  // import time, every time — not only when quality looks off.
  it("labels a .wav import as beta on success", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("take.wav", [1, 2]);
    fireEvent.change(fileInput(), { target: { files: [file] } });
    await screen.findByText(/transcribed from audio \(beta\)/);
  });

  it("routes a .wav file to import_audio_file with its bytes", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("recording.wav", [0x52, 0x49, 0x46, 0x46]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_audio_file", {
        sourceFilename: "recording.wav",
        bytes: [0x52, 0x49, 0x46, 0x46],
      }),
    );
    await screen.findByText(/Imported "recording"/);
  });

  it("routes a .mp3 file to import_audio_file too", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("take.mp3", [9, 9, 9]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "import_audio_file",
        expect.objectContaining({ sourceFilename: "take.mp3" }),
      ),
    );
  });

  it("does not warn about quality for a clean monophonic import", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("recording.wav", [1]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/Imported "recording"/);
    expect(screen.queryByText(/polyphonic/i)).toBeNull();
    expect(screen.queryByText(/approximate/i)).toBeNull();
  });

  it("shows a calm quality banner when the input looks polyphonic", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "import_audio_file")
        return Promise.resolve({
          ...AUDIO_RESULT,
          polyphony: 0.3,
          polyphonic: true,
          uncertain_count: 1,
          verdict: "borderline",
        });
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("band.wav", [1]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/sounds polyphonic/i);
    // The honesty counts ride every quality banner, not just full-mix.
    expect(screen.getByText(/Caught 5 notes, 1 uncertain/)).toBeInTheDocument();
    // Dismissible.
    fireEvent.click(screen.getByLabelText("Dismiss"));
    await waitFor(() =>
      expect(screen.queryByText(/sounds polyphonic/i)).toBeNull(),
    );
  });

  // #331: a weak-but-single-line take (mono verdict, low confidence) must
  // still get its calm "may be approximate" note — the banner gate is
  // verdict OR confidence, not verdict alone. Fails if the low-confidence
  // half of the gate is dropped.
  it("warns on a low-confidence mono transcription", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "import_audio_file")
        return Promise.resolve({
          ...AUDIO_RESULT,
          mean_confidence: 0.3,
          low_confidence: true,
          uncertain_count: 3,
        });
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("faint.wav", [1]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/This transcription may be approximate/);
    expect(screen.getByText(/Caught 5 notes, 3 uncertain/)).toBeInTheDocument();
  });

  // #331 AC1: a full-mix verdict must say so in plain words and carry the
  // honesty counts — never present a full-mix transcription as clean. Fails
  // if the verdict stops selecting the full-mix copy or the counts vanish.
  it("names a full mix and shows the honesty counts on its banner", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "import_audio_file")
        return Promise.resolve({
          ...AUDIO_RESULT,
          entry: { ...ENTRY, title: "band", source_filename: "band.wav" },
          note_count: 12,
          polyphony: 1.0,
          polyphonic: true,
          uncertain_count: 5,
          verdict: "full_mix",
        });
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("band.wav", [1]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(
      /sounds like a full mix — the notes may be approximate/i,
    );
    expect(screen.getByText(/Caught 12 notes, 5 uncertain/)).toBeTruthy();
  });

  // #336 — the VA saw NO loading message during .wav imports even with the
  // #323 backend fix in her build: if event delivery loses its race against
  // the invoke's resolution, the listener is already gone and nothing ever
  // paints. The indicator must not DEPEND on events: it seeds the moment the
  // import starts (worst case: events never fire) and clears when it ends.
  it("shows the transcribing message even when no progress event ever arrives", async () => {
    let resolveImport: ((v: unknown) => void) | undefined;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "import_audio_file")
        return new Promise((res) => {
          resolveImport = res;
        });
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("take.wav", [1, 2]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    // Mid-import, zero events delivered: the loading message still shows.
    await screen.findByText("Listening for notes…");

    resolveImport?.(AUDIO_RESULT);
    await waitFor(() =>
      expect(screen.queryByText("Listening for notes…")).toBeNull(),
    );
  });

  // #336, round three — the VA's sample recording is small enough that the
  // backend can resolve before the seeded indicator ever reaches the screen:
  // seed-then-clear inside one task paints ZERO frames, which is exactly
  // "no loading message appeared" from three consecutive VA runs. Once shown,
  // the in-progress state must survive a human-perceivable minimum even when
  // the import finishes instantly.
  it("keeps the loading message up for a perceivable minimum when the import finishes instantly", async () => {
    vi.useFakeTimers();
    try {
      render(<ScoreDropZone />);
      const file = fileWithBytes("take.wav", [1, 2]);
      fireEvent.change(fileInput(), { target: { files: [file] } });

      // Settle the (instantly-resolving) import. 50ms of fake time gives a
      // cold worker's microtask chain room while staying far inside the hold.
      await act(() => vi.advanceTimersByTimeAsync(50));
      // The backend already returned, yet the message is still on screen…
      expect(screen.getByText("Listening for notes…")).toBeInTheDocument();
      expect(screen.queryByText(/transcribed from audio/)).toBeNull();

      // …and stays through most of the hold window…
      await act(() => vi.advanceTimersByTimeAsync(900));
      expect(screen.getByText("Listening for notes…")).toBeInTheDocument();

      // …then hands over to the imported banner once the hold elapses.
      await act(() => vi.advanceTimersByTimeAsync(400));
      expect(screen.queryByText("Listening for notes…")).toBeNull();
      expect(
        screen.getByText(/transcribed from audio \(beta\)/),
      ).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  // The PDF/OMR branch has the identical zero-frame invisibility class —
  // its hold is pinned separately so it can't be silently dropped.
  it("keeps the PDF reading message up for the same perceivable minimum", async () => {
    vi.useFakeTimers();
    try {
      render(<ScoreDropZone />);
      const file = fileWithBytes("etude.pdf", [0x25, 0x50, 0x44, 0x46]);
      fireEvent.change(fileInput(), { target: { files: [file] } });

      // OMR resolved instantly, yet the seeded message is still on screen…
      await act(() => vi.advanceTimersByTimeAsync(50));
      expect(screen.getByText("Reading the notes…")).toBeInTheDocument();
      expect(screen.queryByText(/read from a scan/i)).toBeNull();

      // …through most of the hold window…
      await act(() => vi.advanceTimersByTimeAsync(900));
      expect(screen.getByText("Reading the notes…")).toBeInTheDocument();

      // …then the flow proceeds to the provenance note + import.
      await act(() => vi.advanceTimersByTimeAsync(400));
      expect(screen.queryByText("Reading the notes…")).toBeNull();
      expect(screen.getByText(/read from a scan/i)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  // The hold is a minimum, not an addend: a transcription that already ran
  // longer than the window hands over to the banner the moment it resolves.
  it("does not extend a slow import beyond its own duration", async () => {
    // Fake `performance` too, so the import's elapsed time is the fake
    // clock's 2000ms rather than the real microseconds the test took.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    try {
      let resolveImport: ((v: unknown) => void) | undefined;
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === "import_audio_file")
          return new Promise((res) => {
            resolveImport = res;
          });
        if (cmd === "get_score")
          return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
        return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
      });
      render(<ScoreDropZone importFeedbackMinMs={0} />);
      const file = fileWithBytes("take.wav", [1]);
      fireEvent.change(fileInput(), { target: { files: [file] } });

      // 2s of transcription — already past the 1.2s hold window.
      await act(() => vi.advanceTimersByTimeAsync(2000));
      expect(screen.getByText("Listening for notes…")).toBeInTheDocument();

      // The banner must arrive on resolution alone, no further timers.
      resolveImport?.(AUDIO_RESULT);
      await act(async () => {});
      expect(
        screen.getByText(/transcribed from audio \(beta\)/),
      ).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  // A tester who sees nothing retries fast. If a second file is dropped
  // while the first import is still holding its indicator, the first's
  // completion must not clobber the second's in-progress UI (review MF1).
  it("a second drop during the first's hold keeps its own indicator", async () => {
    vi.useFakeTimers();
    try {
      // First import resolves instantly (enters its hold); the second pends.
      let audioCalls = 0;
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === "import_audio_file") {
          audioCalls += 1;
          return audioCalls === 1
            ? Promise.resolve(AUDIO_RESULT)
            : new Promise(() => {});
        }
        if (cmd === "get_score")
          return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
        return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
      });
      render(<ScoreDropZone importFeedbackMinMs={0} />);
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("first.wav", [1])] },
      });
      await act(() => vi.advanceTimersByTimeAsync(50));
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("second.wav", [2])] },
      });
      await act(() => vi.advanceTimersByTimeAsync(50));
      expect(screen.getByText("Listening for notes…")).toBeInTheDocument();

      // The first import's hold elapses mid-second-import: its stale
      // completion stays silent — the second's indicator survives.
      await act(() => vi.advanceTimersByTimeAsync(1300));
      expect(screen.getByText("Listening for notes…")).toBeInTheDocument();
      expect(screen.getByText(/Transcribing second\.wav/)).toBeInTheDocument();
      expect(screen.queryByText(/Imported "recording"/)).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  // The hold applies to success only — a failed import reports immediately,
  // with no in-progress message lingering over the error.
  it("shows an import failure immediately, without holding the indicator", async () => {
    vi.useFakeTimers();
    try {
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === "import_audio_file")
          return Promise.reject(new Error("Couldn't decode that recording"));
        return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
      });
      render(<ScoreDropZone importFeedbackMinMs={0} />);
      const file = fileWithBytes("broken.wav", [1]);
      fireEvent.change(fileInput(), { target: { files: [file] } });

      await act(() => vi.advanceTimersByTimeAsync(50));
      expect(
        screen.getByText(/Couldn't decode that recording/),
      ).toBeInTheDocument();
      expect(screen.queryByText("Listening for notes…")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("renders progress from import-progress events during audio import", async () => {
    // Hold the import open so we can fire a progress event mid-flight.
    let resolveImport: (v: unknown) => void = () => {};
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "import_audio_file")
        return new Promise((res) => {
          resolveImport = res;
        });
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });

    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("recording.wav", [1]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    // Wait for the component to register its progress listener.
    await waitFor(() => expect(progressHandlers.length).toBeGreaterThan(0));
    progressHandlers[0]({ payload: { stage: "transcribing", pct: 45 } });

    await screen.findByText(/Listening for notes/);
    expect(screen.getByText("45%")).toBeTruthy();

    resolveImport(AUDIO_RESULT);
    await screen.findByText(/Imported "recording"/);
  });

  it("imports a single-part MusicXML file directly (part 0, no picker)", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("song.musicxml", [0x3c, 0x3f, 0x78]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_musicxml_file", {
        sourceFilename: "song.musicxml",
        bytes: [0x3c, 0x3f, 0x78],
        partIndex: 0,
      }),
    );
    await screen.findByText(/Imported "scales"/);
    // No picker shown for a single-part score.
    expect(screen.queryByText(/which one do you want to read/i)).toBeNull();
  });

  it("asks which part to read for a multi-part MusicXML, then imports the choice", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_score_parts")
        return Promise.resolve(["Trumpet", "Trombone"]);
      if (cmd === "import_musicxml_file") return Promise.resolve(ENTRY);
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("duet.xml", [1, 2, 3]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    // Picker appears with both part names; nothing imported yet.
    await screen.findByText(/which one do you want to read/i);
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "import_musicxml_file",
      expect.anything(),
    );

    // Pick the second part → imports with partIndex 1.
    fireEvent.click(screen.getByText("Trombone"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_musicxml_file", {
        sourceFilename: "duet.xml",
        bytes: [1, 2, 3],
        partIndex: 1,
      }),
    );
    await screen.findByText(/Imported "scales"/);
  });

  it("recognizes a single-part PDF and imports it via the shared MusicXML path", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("etude.pdf", [0x25, 0x50, 0x44, 0x46]); // %PDF
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("recognize_pdf_score", {
        sourceFilename: "etude.pdf",
        bytes: [0x25, 0x50, 0x44, 0x46],
      }),
    );
    // The recognized MusicXML is fed back through the SAME import command,
    // encoded as UTF-8 bytes — OMR reuses the MusicXML import path.
    const xmlBytes = Array.from(new TextEncoder().encode("<score-partwise/>"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_musicxml_file", {
        sourceFilename: "etude.pdf",
        bytes: xmlBytes,
        partIndex: 0,
      }),
    );
    // The "read from a scan" provenance note is always shown.
    await screen.findByText(/read from a scan/i);
  });

  it("asks which part to read for a multi-part PDF, then imports the choice", async () => {
    const xml = "<score-partwise>duet</score-partwise>";
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "recognize_pdf_score")
        return Promise.resolve({
          music_xml: xml,
          parts: ["Right Hand", "Left Hand"],
          from_scan: true,
          low_content: false,
        });
      if (cmd === "import_musicxml_file") return Promise.resolve(ENTRY);
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("duet.pdf", [0x25, 0x50, 0x44, 0x46]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    // Same picker as MusicXML import; nothing imported until a part is chosen.
    await screen.findByText(/which one do you want to read/i);
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "import_musicxml_file",
      expect.anything(),
    );

    fireEvent.click(screen.getByText("Left Hand"));
    const xmlBytes = Array.from(new TextEncoder().encode(xml));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_musicxml_file", {
        sourceFilename: "duet.pdf",
        bytes: xmlBytes,
        partIndex: 1,
      }),
    );
  });

  it("warns plainly when a PDF scan yielded almost nothing", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "recognize_pdf_score")
        return Promise.resolve({
          music_xml: "<score-partwise/>",
          parts: ["Piano"],
          from_scan: true,
          low_content: true,
        });
      if (cmd === "import_musicxml_file") return Promise.resolve(ENTRY);
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("blurry.pdf", [0x25, 0x50, 0x44, 0x46]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/barely read any music/i);
  });

  it("surfaces the backend's calm message when PDF import is disabled", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "recognize_pdf_score")
        return Promise.reject(
          new Error(
            "Reading sheet-music PDFs is an experimental feature that isn't enabled in this build yet.",
          ),
        );
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("etude.pdf", [0x25, 0x50, 0x44, 0x46]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/experimental feature that isn't enabled/i);
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "import_musicxml_file",
      expect.anything(),
    );
  });

  it("guides the user to re-export a compressed .mxl file", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = fileWithBytes("song.mxl", [0x50, 0x4b, 0x03, 0x04]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/Compressed .mxl files aren't supported yet/);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("rejects an unsupported extension without invoking anything", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    const file = new File(["nope"], "notes.txt");
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/Unsupported format/);
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});

// Post-review hardening (test-audit survivors S1–S3): the staleness guards
// on the ERROR and QUALITY paths existed in code but nothing pinned them,
// and PDF error immediacy was only asserted under a masking minMs=0.
describe("staleness guards on error/quality paths (#336 audit S1–S3)", () => {
  // S1: a slow FAILING import must not clobber a newer drop's indicator
  // with its stale error. Kills the mutant that drops isCurrentImport()
  // around the audio catch's setStatus/setError.
  it("a stale rejection stays silent while a newer drop is live", async () => {
    vi.useFakeTimers();
    try {
      let audioCalls = 0;
      let rejectFirst!: (e: Error) => void;
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === "import_audio_file") {
          audioCalls += 1;
          return audioCalls === 1
            ? new Promise((_r, rej) => {
                rejectFirst = rej;
              })
            : new Promise(() => {}); // second import pends forever
        }
        return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
      });
      render(<ScoreDropZone importFeedbackMinMs={0} />);
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("first.wav", [1])] },
      });
      await act(() => vi.advanceTimersByTimeAsync(50));
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("second.wav", [2])] },
      });
      await act(() => vi.advanceTimersByTimeAsync(50));

      rejectFirst(new Error("decoder exploded"));
      await act(() => vi.advanceTimersByTimeAsync(50));

      // The stale failure must neither surface its error nor tear down
      // the live import's indicator.
      expect(screen.queryByText(/decoder exploded/)).toBeNull();
      expect(screen.getByText("Listening for notes…")).toBeInTheDocument();
      expect(screen.getByText(/Transcribing second\.wav/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  // S2: a stale import's QUALITY banner must not land on a newer drop.
  // Kills the mutant that hoists setQuality out of the staleness guard.
  it("a stale polyphonic result's banner stays off a newer drop", async () => {
    vi.useFakeTimers();
    try {
      let audioCalls = 0;
      let resolveFirst!: (v: unknown) => void;
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === "import_audio_file") {
          audioCalls += 1;
          return audioCalls === 1
            ? new Promise((res) => {
                resolveFirst = res;
              })
            : new Promise(() => {});
        }
        if (cmd === "get_score")
          return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
        return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
      });
      render(<ScoreDropZone importFeedbackMinMs={0} />);
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("first.wav", [1])] },
      });
      await act(() => vi.advanceTimersByTimeAsync(50));
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("second.wav", [2])] },
      });
      await act(() => vi.advanceTimersByTimeAsync(50));

      resolveFirst({ ...AUDIO_RESULT, polyphonic: true });
      await act(() => vi.advanceTimersByTimeAsync(100));

      expect(screen.queryByText(/sounds polyphonic/i)).toBeNull();
      expect(screen.getByText(/Transcribing second\.wav/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  // S3: PDF failures report immediately even at the real (nonzero) hold —
  // the audio twin of this test exists; the PDF branch was only covered
  // under minMs=0, which masks an accidental hold on its error path.
  it("a PDF failure reports immediately at the real hold value", async () => {
    vi.useFakeTimers();
    try {
      mockInvoke.mockImplementation((cmd: string) =>
        cmd === "recognize_pdf_score"
          ? Promise.reject(new Error("scan too blurry"))
          : Promise.reject(new Error(`no mock for invoke("${cmd}")`)),
      );
      render(<ScoreDropZone importFeedbackMinMs={1200} />);
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("etude.pdf", [1])] },
      });
      // Well inside the 1200ms hold window: the error is already up and
      // the in-progress message is already gone.
      await act(() => vi.advanceTimersByTimeAsync(60));
      expect(screen.getByText(/scan too blurry/)).toBeInTheDocument();
      expect(screen.queryByText(/Reading etude\.pdf/)).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

// #336, the instrumentation round: after six VA runs with no .wav loading
// message and every test green, the founder's plan is to log the four
// moments of the feedback lifecycle from inside the real shell. These pin
// the wiring — each moment reaches `frontend_breadcrumb`, in order, with
// the entry path recorded — so the diagnostic can't silently rot before
// the VA run that needs it.
describe("import-feedback breadcrumbs (#336)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installInvokeMock();
  });

  const breadcrumbs = () =>
    mockInvoke.mock.calls
      .filter((c) => c[0] === "frontend_breadcrumb")
      .map((c) => (c[1] as { message: string }).message);

  it("logs seed → first frame → hold done → cleared, in order, for a picker .wav", async () => {
    vi.useFakeTimers();
    try {
      render(<ScoreDropZone />);
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("take.wav", [1, 2])] },
      });

      // Seed logs with the import running; the rAF first-frame probe
      // (fake rAF fires at ~16ms, second sample ~32ms) lands next. The
      // hold hasn't elapsed, so neither closing moment may have logged.
      await act(() => vi.advanceTimersByTimeAsync(50));
      expect(breadcrumbs()).toEqual([
        "import feedback #1: seeded stage=transcribing via file-picker (.wav)",
        expect.stringMatching(/^import feedback #1: first frame — /),
      ]);

      // The hold elapses → hold done, then cleared, complete the four.
      await act(() => vi.advanceTimersByTimeAsync(1300));
      const lines = breadcrumbs();
      expect(lines).toHaveLength(4);
      expect(lines[2]).toMatch(/^import feedback #1: hold done after \d+ms$/);
      expect(lines[3]).toBe("import feedback #1: cleared (success)");
    } finally {
      vi.useRealTimers();
    }
  });

  // The probe must observe the REAL indicator, not just emit a line: with
  // the import held in flight and the message provably on screen, the
  // logged frame line carries the indicator's visible label and computed
  // style. Kills a probe that runs synchronously at seed time (only ever
  // sees pre-commit DOM) or one that hardcodes its report.
  it("the first-frame probe reports the indicator it actually observes", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "frontend_breadcrumb") return Promise.resolve();
      if (cmd === "import_audio_file") return new Promise(() => {}); // in flight
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone />);
    fireEvent.change(fileInput(), {
      target: { files: [fileWithBytes("take.wav", [1])] },
    });

    await screen.findByText("Listening for notes…");
    await waitFor(() =>
      expect(
        breadcrumbs().some((l) =>
          /first frame — (?:.*second frame — )?"Listening for notes…" x=-?\d+ y=-?\d+ w=\d+ h=\d+ display=block visibility=visible$/.test(
            l,
          ),
        ),
      ).toBe(true),
    );
  });

  it("records the drag-drop entry path when a .wav is dropped", async () => {
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    fireEvent.drop(screen.getByTestId("score-drop-zone"), {
      dataTransfer: { files: [fileWithBytes("take.wav", [1])] },
    });

    await waitFor(() =>
      expect(breadcrumbs()).toContain(
        "import feedback #1: seeded stage=transcribing via drag-drop (.wav)",
      ),
    );
    // The first-frame probe fires on this entry path too.
    await waitFor(() =>
      expect(breadcrumbs().some((l) => l.includes("first frame"))).toBe(true),
    );
  });

  it("logs cleared (error) — and no hold line — when the import fails", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "frontend_breadcrumb") return Promise.resolve();
      if (cmd === "import_audio_file")
        return Promise.reject(new Error("Couldn't decode that recording"));
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    fireEvent.change(fileInput(), {
      target: { files: [fileWithBytes("broken.wav", [1])] },
    });

    await screen.findByText(/Couldn't decode that recording/);
    await waitFor(() =>
      expect(breadcrumbs()).toContain("import feedback #1: cleared (error)"),
    );
    expect(breadcrumbs().some((l) => l.includes("hold done"))).toBe(false);
    // The first-frame probe fires on this path too.
    await waitFor(() =>
      expect(breadcrumbs().some((l) => l.includes("first frame"))).toBe(true),
    );
  });

  it("attributes a superseded import's cleared line to it, with its own ending", async () => {
    vi.useFakeTimers();
    try {
      let audioCalls = 0;
      mockInvoke.mockImplementation((cmd: string) => {
        if (cmd === "frontend_breadcrumb") return Promise.resolve();
        if (cmd === "import_audio_file") {
          audioCalls += 1;
          return audioCalls === 1
            ? Promise.resolve(AUDIO_RESULT)
            : new Promise(() => {}); // second import pends forever
        }
        if (cmd === "get_score")
          return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
        return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
      });
      render(<ScoreDropZone />);
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("first.wav", [1])] },
      });
      await act(() => vi.advanceTimersByTimeAsync(50));
      fireEvent.change(fileInput(), {
        target: { files: [fileWithBytes("second.wav", [2])] },
      });

      // The first import's hold elapses while the second still pends: its
      // writes were silenced, and its cleared line must say so — while
      // staying attributable (#1) and keeping its own ending.
      await act(() => vi.advanceTimersByTimeAsync(1300));
      expect(breadcrumbs()).toContain(
        "import feedback #1: cleared (superseded, was success)",
      );
      expect(breadcrumbs()).toContain(
        "import feedback #2: seeded stage=transcribing via file-picker (.wav)",
      );
      expect(breadcrumbs()).not.toContain("import feedback #1: cleared (success)");
    } finally {
      vi.useRealTimers();
    }
  });

  it("logs the full lifecycle for a PDF import, with its own stage", async () => {
    vi.useFakeTimers();
    try {
      render(<ScoreDropZone />);
      fireEvent.change(fileInput(), {
        target: {
          files: [fileWithBytes("etude.pdf", [0x25, 0x50, 0x44, 0x46])],
        },
      });

      await act(() => vi.advanceTimersByTimeAsync(50));
      expect(breadcrumbs()).toEqual([
        "import feedback #1: seeded stage=reading-notes via file-picker (.pdf)",
        expect.stringMatching(/^import feedback #1: first frame — /),
      ]);

      await act(() => vi.advanceTimersByTimeAsync(1300));
      const lines = breadcrumbs();
      expect(lines).toHaveLength(4);
      expect(lines[2]).toMatch(/^import feedback #1: hold done after \d+ms$/);
      expect(lines[3]).toBe("import feedback #1: cleared (success)");
    } finally {
      vi.useRealTimers();
    }
  });

  it("logs cleared (error) — and no hold line — when PDF recognition fails", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "frontend_breadcrumb") return Promise.resolve();
      if (cmd === "recognize_pdf_score")
        return Promise.reject(new Error("scan too blurry"));
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone importFeedbackMinMs={0} />);
    fireEvent.change(fileInput(), {
      target: { files: [fileWithBytes("etude.pdf", [1])] },
    });

    await screen.findByText(/scan too blurry/);
    await waitFor(() =>
      expect(breadcrumbs()).toContain("import feedback #1: cleared (error)"),
    );
    expect(breadcrumbs().some((l) => l.includes("hold done"))).toBe(false);
    // The first-frame probe fires on this path too.
    await waitFor(() =>
      expect(breadcrumbs().some((l) => l.includes("first frame"))).toBe(true),
    );
  });
});
