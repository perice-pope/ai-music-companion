import { useRef, useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";

const MIDI_EXTS = ["mid", "midi"];
const MUSICXML_EXTS = ["musicxml", "mxl", "xml"];
const VALID_EXTS = [...MUSICXML_EXTS, ...MIDI_EXTS];

export default function ScoreDropZone() {
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const importMidiFromFile = usePracticeStore((s) => s.importMidiFromFile);

  // Route a chosen file by extension. MIDI is fully wired: read the bytes
  // and hand them to the backend, which parses → MusicXML → library. The
  // MusicXML path still needs frontend metadata parsing (a later slice), so
  // it surfaces an honest "not yet" rather than silently failing.
  const handleFile = async (file: File) => {
    setError(null);
    setStatus(null);

    const ext = file.name.split(".").pop()?.toLowerCase();
    if (!ext || !VALID_EXTS.includes(ext)) {
      setError("Unsupported format. Use .musicxml, .mxl, .xml, .mid, or .midi files.");
      return;
    }

    if (MIDI_EXTS.includes(ext)) {
      try {
        setStatus(`Importing ${file.name}…`);
        const buffer = await file.arrayBuffer();
        const bytes = Array.from(new Uint8Array(buffer));
        const entry = await importMidiFromFile(file.name, bytes);
        setStatus(`Imported "${entry.title}".`);
      } catch (err) {
        setStatus(null);
        setError(`${err instanceof Error ? err.message : err}`);
      }
      return;
    }

    // MusicXML/.xml/.mxl: backend command exists, but the frontend metadata
    // extraction it expects isn't built yet.
    setError("MusicXML import isn't wired up yet — MIDI files work today.");
  };

  const handleDrop = async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragging(false);
    const files = e.dataTransfer.files;
    if (files.length === 0) return;
    await handleFile(files[0]);
  };

  const handleBrowse = () => {
    fileInputRef.current?.click();
  };

  const handleFileInput = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.currentTarget.files;
    if (!files || files.length === 0) return;
    await handleFile(files[0]);
  };

  return (
    <div className="rounded-lg border-2 border-dashed border-gray-600 bg-gray-800 p-8">
      <div
        onDrop={handleDrop}
        onDragOver={(e) => {
          e.preventDefault();
          setIsDragging(true);
        }}
        onDragLeave={() => setIsDragging(false)}
        className={`rounded-lg border-2 border-dashed p-8 text-center transition ${
          isDragging
            ? "border-blue-400 bg-blue-900/20"
            : "border-gray-600 bg-transparent"
        }`}
      >
        <div className="text-4xl mb-4">🎵</div>
        <h3 className="text-lg font-semibold">
          {isDragging ? "Drop your score here" : "Drag a score here"}
        </h3>
        <p className="mt-2 text-sm text-gray-400">
          Supports .musicxml, .mxl, .xml, .mid, and .midi files
        </p>

        <div className="mt-6 flex justify-center gap-4">
          <button
            onClick={handleBrowse}
            className="rounded bg-blue-600 px-6 py-2 font-semibold hover:bg-blue-700 transition"
          >
            Choose file…
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept=".musicxml,.mxl,.xml,.mid,.midi"
            onChange={handleFileInput}
            className="hidden"
          />
        </div>
      </div>

      {status && (
        <div className="mt-4 rounded bg-blue-900/20 border border-blue-500 p-3 text-blue-200 text-sm">
          {status}
        </div>
      )}

      {error && (
        <div className="mt-4 rounded bg-red-900/20 border border-red-500 p-3 text-red-200 text-sm">
          {error}
        </div>
      )}
    </div>
  );
}
