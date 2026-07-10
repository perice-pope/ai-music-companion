import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
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
};

function installInvokeMock() {
  mockInvoke.mockImplementation((cmd: string) => {
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
    expect(
      screen.getByText(
        /Reliable: \.musicxml, \.xml, \.mid, \.midi scores/,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Beta, still rough:\s*\.pdf scans, \.wav\/\.mp3 recordings/),
    ).toBeInTheDocument();
  });

  // #337 S1 AC2 (second clause): a .wav import shows its beta label AT
  // import time, every time — not only when quality looks off.
  it("labels a .wav import as beta on success", async () => {
    render(<ScoreDropZone />);
    const file = fileWithBytes("take.wav", [1, 2]);
    fireEvent.change(fileInput(), { target: { files: [file] } });
    await screen.findByText(/transcribed from audio \(beta\)/);
  });

  it("routes a .wav file to import_audio_file with its bytes", async () => {
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
          polyphony: 0.5,
          polyphonic: true,
        });
      if (cmd === "get_score")
        return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
      return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
    });
    render(<ScoreDropZone />);
    const file = fileWithBytes("band.wav", [1]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/sounds polyphonic/i);
    // Dismissible.
    fireEvent.click(screen.getByLabelText("Dismiss"));
    await waitFor(() =>
      expect(screen.queryByText(/sounds polyphonic/i)).toBeNull(),
    );
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

    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
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
    render(<ScoreDropZone />);
    const file = fileWithBytes("etude.pdf", [0x25, 0x50, 0x44, 0x46]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/experimental feature that isn't enabled/i);
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "import_musicxml_file",
      expect.anything(),
    );
  });

  it("guides the user to re-export a compressed .mxl file", async () => {
    render(<ScoreDropZone />);
    const file = fileWithBytes("song.mxl", [0x50, 0x4b, 0x03, 0x04]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/Compressed .mxl files aren't supported yet/);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("rejects an unsupported extension without invoking anything", async () => {
    render(<ScoreDropZone />);
    const file = new File(["nope"], "notes.txt");
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/Unsupported format/);
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
