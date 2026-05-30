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

  it("does not call import_midi_file for a MusicXML file (not yet wired)", async () => {
    render(<ScoreDropZone />);
    const file = new File(["<score/>"], "song.musicxml");
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/MusicXML import isn't wired up yet/);
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "import_midi_file",
      expect.anything(),
    );
  });

  it("rejects an unsupported extension without invoking anything", async () => {
    render(<ScoreDropZone />);
    const file = new File(["nope"], "notes.txt");
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await screen.findByText(/Unsupported format/);
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
