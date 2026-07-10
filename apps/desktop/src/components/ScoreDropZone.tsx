import { useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { usePracticeStore, type ImportedAudio } from "../stores/practiceStore";

const MIDI_EXTS = ["mid", "midi"];
// Uncompressed MusicXML is plain text we can parse directly. Compressed `.mxl`
// is a ZIP container — accepted in the picker so we can give a clear "re-export
// uncompressed" hint rather than a confusing failure, but not parsed yet.
const MUSICXML_PLAIN_EXTS = ["musicxml", "xml"];
const MUSICXML_COMPRESSED_EXTS = ["mxl"];
const MUSICXML_EXTS = [...MUSICXML_PLAIN_EXTS, ...MUSICXML_COMPRESSED_EXTS];
// Open Question 5 (founder, 2026-05-30): ship .wav + .mp3 in v1. The Rust
// decoder (Symphonia) supports more (.m4a/.flac); widening is a one-line change
// here when we want it.
const AUDIO_EXTS = ["wav", "mp3"];
// Sheet-music PDF → on-device OMR → MusicXML. Experimental beta: the backend
// gates it (AMC_ENABLE_PDF_OMR) and returns a calm message when it's off, so we
// can accept the drop and let the explanation come from there.
const PDF_EXTS = ["pdf"];
const VALID_EXTS = [...MUSICXML_EXTS, ...MIDI_EXTS, ...AUDIO_EXTS, ...PDF_EXTS];

/** `import-progress` event payload from the backend (audio import only). */
interface ImportProgress {
  stage: string;
  pct: number;
}

const STAGE_LABELS: Record<string, string> = {
  decoding: "Reading audio…",
  transcribing: "Listening for notes…",
  converting: "Building the score…",
  // PDF → OMR stages.
  rasterizing: "Scanning the page…",
  "reading-notes": "Reading the notes…",
  done: "Finishing up…",
};

export default function ScoreDropZone() {
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  const [quality, setQuality] = useState<ImportedAudio | null>(null);
  // Set after a PDF is read by OMR — a calm "this came from a scan, check it"
  // note that stays visible after import (it's about the score's provenance).
  const [scanNote, setScanNote] = useState<{ lowContent: boolean } | null>(null);
  // A multi-part file (MusicXML or MIDI) waiting for the user to pick which
  // part to read. For MIDI, `midiTrackIndices[i]` is the original track
  // number behind label `parts[i]`; absent for MusicXML (part = list index).
  const [partChoice, setPartChoice] = useState<{
    fileName: string;
    bytes: number[];
    parts: string[];
    midiTrackIndices?: number[];
  } | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const importMidiFromFile = usePracticeStore((s) => s.importMidiFromFile);
  const listMidiParts = usePracticeStore((s) => s.listMidiParts);
  const importAudioFromFile = usePracticeStore((s) => s.importAudioFromFile);
  const listScoreParts = usePracticeStore((s) => s.listScoreParts);
  const importMusicXmlFromFile = usePracticeStore((s) => s.importMusicXmlFromFile);
  const recognizePdfFromFile = usePracticeStore((s) => s.recognizePdfFromFile);

  // Import a (plain) MusicXML file once the part is known: a single-part score
  // imports part 0 immediately; a multi-part score routes through the picker.
  const importMusicXml = async (
    fileName: string,
    bytes: number[],
    partIndex: number,
  ) => {
    try {
      setPartChoice(null);
      setStatus(`Importing ${fileName}…`);
      const entry = await importMusicXmlFromFile(fileName, bytes, partIndex);
      setStatus(`Imported "${entry.title}".`);
    } catch (err) {
      setStatus(null);
      setError(`${err instanceof Error ? err.message : err}`);
    }
  };

  // Route a chosen file by extension. MIDI → backend parse → MusicXML → library.
  // Audio → backend transcribe (basic-pitch) → MIDI → MusicXML → library, with
  // live progress and a calm quality note. MusicXML still needs frontend
  // metadata parsing (a later slice), so it surfaces an honest "not yet".
  const handleFile = async (file: File) => {
    setError(null);
    setStatus(null);
    setQuality(null);
    setScanNote(null);
    setProgress(null);
    setPartChoice(null);

    const ext = file.name.split(".").pop()?.toLowerCase();
    if (!ext || !VALID_EXTS.includes(ext)) {
      setError(
        "Unsupported format. Use .musicxml, .mxl, .xml, .mid, .midi, .wav, or .mp3 files.",
      );
      return;
    }

    if (MIDI_EXTS.includes(ext)) {
      try {
        setStatus(`Importing ${file.name}…`);
        const buffer = await file.arrayBuffer();
        const bytes = Array.from(new Uint8Array(buffer));
        // A band file has several playable tracks — ask which one to
        // practice, exactly like multi-part MusicXML (#337 S1). Conductor
        // and drum tracks are already filtered out by the backend.
        const midiParts = await listMidiParts(bytes);
        if (midiParts.length > 1) {
          setStatus(null);
          setPartChoice({
            fileName: file.name,
            bytes,
            parts: midiParts.map(
              (p) =>
                `${p.name} (${p.note_count} note${p.note_count === 1 ? "" : "s"})`,
            ),
            midiTrackIndices: midiParts.map((p) => p.track_index),
          });
          return;
        }
        const entry = await importMidiFromFile(file.name, bytes);
        setStatus(`Imported "${entry.title}".`);
      } catch (err) {
        setStatus(null);
        setError(`${err instanceof Error ? err.message : err}`);
      }
      return;
    }

    if (AUDIO_EXTS.includes(ext)) {
      let unlisten: (() => void) | undefined;
      try {
        setStatus(`Transcribing ${file.name}…`);
        unlisten = await listen<ImportProgress>("import-progress", (event) => {
          setProgress(event.payload);
        });
        const buffer = await file.arrayBuffer();
        const bytes = Array.from(new Uint8Array(buffer));
        const result = await importAudioFromFile(file.name, bytes);
        setStatus(`Imported "${result.entry.title}".`);
        // Only surface the banner when something looks off — never nag.
        if (result.polyphonic || result.low_confidence) {
          setQuality(result);
        }
      } catch (err) {
        setStatus(null);
        setError(`${err instanceof Error ? err.message : err}`);
      } finally {
        unlisten?.();
        setProgress(null);
      }
      return;
    }

    if (PDF_EXTS.includes(ext)) {
      // PDF → on-device OMR → MusicXML, then the SAME "which part?" picker and
      // import path as MusicXML (OMR is just another front-end producing it).
      let unlisten: (() => void) | undefined;
      try {
        setStatus(`Reading ${file.name}…`);
        unlisten = await listen<ImportProgress>("import-progress", (event) => {
          setProgress(event.payload);
        });
        const buffer = await file.arrayBuffer();
        const bytes = Array.from(new Uint8Array(buffer));
        const recognized = await recognizePdfFromFile(file.name, bytes);
        // OMR is approximate — always surface the "read from a scan" note.
        setScanNote({ lowContent: recognized.low_content });
        const xmlBytes = Array.from(
          new TextEncoder().encode(recognized.music_xml),
        );
        if (recognized.parts.length <= 1) {
          await importMusicXml(file.name, xmlBytes, 0);
        } else {
          setStatus(null);
          setPartChoice({
            fileName: file.name,
            bytes: xmlBytes,
            parts: recognized.parts,
          });
        }
      } catch (err) {
        setStatus(null);
        setError(`${err instanceof Error ? err.message : err}`);
      } finally {
        unlisten?.();
        setProgress(null);
      }
      return;
    }

    if (MUSICXML_COMPRESSED_EXTS.includes(ext)) {
      setError(
        "Compressed .mxl files aren't supported yet. Re-export as uncompressed " +
          "MusicXML (.musicxml or .xml) from your notation app and try again.",
      );
      return;
    }

    // Plain MusicXML (.musicxml / .xml): ask the backend which parts it has.
    // One part → import it straight away; several → show the part picker so the
    // user chooses which line to read and practice.
    try {
      setStatus(`Reading ${file.name}…`);
      const buffer = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(buffer));
      const parts = await listScoreParts(bytes);
      if (parts.length <= 1) {
        await importMusicXml(file.name, bytes, 0);
      } else {
        setStatus(null);
        setPartChoice({ fileName: file.name, bytes, parts });
      }
    } catch (err) {
      setStatus(null);
      setError(`${err instanceof Error ? err.message : err}`);
    }
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

  const qualityMessage = quality?.polyphonic
    ? "This recording sounds polyphonic — basic-pitch works best on a single instrument line, so the transcription may be rough. You can re-record or drop a different file anytime."
    : "This transcription may be approximate — try a closer, single-instrument recording for a cleaner result.";

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
          {isDragging ? "Drop your score here" : "Drag a score or recording here"}
        </h3>
        <p className="mt-2 text-sm text-gray-400">
          Reliable: .musicxml, .xml, .mid, .midi scores · Beta, still rough:
          .pdf scans, .wav/.mp3 recordings
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
            accept=".musicxml,.mxl,.xml,.mid,.midi,.wav,.mp3,.pdf"
            onChange={handleFileInput}
            className="hidden"
          />
        </div>
      </div>

      {progress && (
        <div className="mt-4" role="progressbar" aria-valuenow={progress.pct}>
          <div className="flex justify-between text-xs text-blue-200 mb-1">
            <span>{STAGE_LABELS[progress.stage] ?? "Working…"}</span>
            <span>{progress.pct}%</span>
          </div>
          <div className="h-2 rounded bg-gray-700 overflow-hidden">
            <div
              className="h-full bg-blue-500 transition-all"
              style={{ width: `${progress.pct}%` }}
            />
          </div>
        </div>
      )}

      {status && (
        <div className="mt-4 rounded bg-blue-900/20 border border-blue-500 p-3 text-blue-200 text-sm">
          {status}
        </div>
      )}

      {partChoice && (
        <div className="mt-4 rounded bg-gray-900/60 border border-gray-600 p-4 text-sm">
          <p className="font-semibold text-gray-100">
            This score has several parts. Which one do you want to read and
            practice?
          </p>
          <p className="mt-1 text-gray-400">
            That part appears on screen as your sheet music and drives the
            moving cursor.
          </p>
          <div className="mt-3 flex flex-col gap-2">
            {partChoice.parts.map((name, idx) => (
              <button
                key={`${name}-${idx}`}
                onClick={async () => {
                  if (partChoice.midiTrackIndices) {
                    try {
                      setPartChoice(null);
                      setStatus(`Importing ${partChoice.fileName}…`);
                      const entry = await importMidiFromFile(
                        partChoice.fileName,
                        partChoice.bytes,
                        partChoice.midiTrackIndices[idx],
                      );
                      setStatus(`Imported "${entry.title}".`);
                    } catch (err) {
                      setStatus(null);
                      setError(`${err instanceof Error ? err.message : err}`);
                    }
                  } else {
                    await importMusicXml(
                      partChoice.fileName,
                      partChoice.bytes,
                      idx,
                    );
                  }
                }}
                className="text-left rounded bg-gray-800 hover:bg-blue-700 border border-gray-600 px-3 py-2 transition"
              >
                {name}
              </button>
            ))}
          </div>
          <button
            onClick={() => setPartChoice(null)}
            className="mt-3 text-gray-400 hover:text-gray-200 text-xs"
          >
            Cancel
          </button>
        </div>
      )}

      {quality && (
        <div className="mt-4 rounded bg-amber-900/20 border border-amber-500 p-3 text-amber-200 text-sm flex items-start justify-between gap-3">
          <span>{qualityMessage}</span>
          <button
            onClick={() => setQuality(null)}
            aria-label="Dismiss"
            className="text-amber-300 hover:text-amber-100 font-bold"
          >
            ✕
          </button>
        </div>
      )}

      {scanNote && (
        <div className="mt-4 rounded bg-amber-900/20 border border-amber-500 p-3 text-amber-200 text-sm flex items-start justify-between gap-3">
          <span>
            {scanNote.lowContent
              ? "We could barely read any music from that PDF — it may be a photo, a very dense page, or not sheet music. Double-check the result, or try a clearer scan."
              : "These notes were read from a scan, so double-check they match your sheet music — especially rhythms, accidentals, and ties."}
          </span>
          <button
            onClick={() => setScanNote(null)}
            aria-label="Dismiss scan note"
            className="text-amber-300 hover:text-amber-100 font-bold"
          >
            ✕
          </button>
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
