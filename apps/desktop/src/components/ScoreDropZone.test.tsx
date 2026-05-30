import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import ScoreDropZone from "./ScoreDropZone";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
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

function installInvokeMock() {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "import_midi_file") return Promise.resolve(ENTRY);
    if (cmd === "get_score")
      return Promise.resolve({ entry: ENTRY, music_xml: "<score/>" });
    return Promise.reject(new Error(`no mock for invoke("${cmd}")`));
  });
}

// jsdom's File doesn't implement arrayBuffer(); the real Tauri webview
// (Chromium/WebKit) does. Build a File with a working arrayBuffer for tests.
function midiFile(name: string, bytes: number[]): File {
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
    installInvokeMock();
  });

  it("routes a .mid file to import_midi_file with its bytes", async () => {
    render(<ScoreDropZone />);
    const file = midiFile("scales.mid", [0x4d, 0x54, 0x68, 0x64]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("import_midi_file", {
        sourceFilename: "scales.mid",
        bytes: [0x4d, 0x54, 0x68, 0x64],
      }),
    );
    // Confirms it surfaced the imported title.
    await screen.findByText(/Imported "scales"/);
  });

  it("routes a .midi file to import_midi_file too", async () => {
    render(<ScoreDropZone />);
    const file = midiFile("piece.midi", [1, 2, 3]);
    fireEvent.change(fileInput(), { target: { files: [file] } });

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "import_midi_file",
        expect.objectContaining({ sourceFilename: "piece.midi" }),
      ),
    );
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
