import { useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { usePracticeStore, type ImportedAudio } from "../stores/practiceStore";
import {
  importClearedBreadcrumb,
  importHoldBreadcrumb,
  importSeededBreadcrumb,
  scheduleImportFirstFrameProbe,
  type ImportEntryPath,
  type ImportOutcome,
} from "../lib/importBreadcrumbs";
import { sendBreadcrumb } from "../lib/positionBreadcrumbs";

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

// #336, round three: the VA watched .wav imports produce sheet music with no
// loading message visible — a short recording transcribes faster than the
// seeded indicator (#342) can reach the screen, so seed-then-clear paints
// zero perceivable frames. Once shown, the in-progress state stays up at
// least this long, however fast the backend finishes.
const IMPORT_FEEDBACK_MIN_MS = 1200;

/** Resolves once the indicator shown at `shownAt` has been on screen for
 *  `minMs`. No-op when the import already took longer. */
function holdImportFeedback(shownAt: number, minMs: number): Promise<void> {
  const remaining = minMs - (performance.now() - shownAt);
  return remaining > 0
    ? new Promise((resolve) => setTimeout(resolve, remaining))
    : Promise.resolve();
}

interface ScoreDropZoneProps {
  /** Test seam only — routing/banner tests pass 0 so they don't sit through
   *  the real hold. The product default is `IMPORT_FEEDBACK_MIN_MS`. */
  importFeedbackMinMs?: number;
}

export default function ScoreDropZone({
  importFeedbackMinMs = IMPORT_FEEDBACK_MIN_MS,
}: ScoreDropZoneProps = {}) {
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  const [quality, setQuality] = useState<ImportedAudio | null>(null);
  // Set after a PDF is read by OMR — a calm "this came from a scan, check it"
  // note that stays visible after import (it's about the score's provenance).
  const [scanNote, setScanNote] = useState<{ lowContent: boolean } | null>(
    null,
  );
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
  // Scopes the #336 first-frame probe's DOM query to this drop zone.
  const rootRef = useRef<HTMLDivElement>(null);
  // Single-shot guard for the part-picker buttons: two clicks in one frame
  // both see the pre-dismissal partChoice and would import twice (review S7).
  const partImportInFlight = useRef(false);
  // Monotonic id of the latest import. The #336 hold keeps an import's
  // completion writes pending for up to IMPORT_FEEDBACK_MIN_MS — long enough
  // for the user to drop a second file — so a finished import only writes
  // status/progress if it is still the current one.
  const importSeq = useRef(0);
  const importMidiFromFile = usePracticeStore((s) => s.importMidiFromFile);
  const listMidiParts = usePracticeStore((s) => s.listMidiParts);
  const importAudioFromFile = usePracticeStore((s) => s.importAudioFromFile);
  const listScoreParts = usePracticeStore((s) => s.listScoreParts);
  const importMusicXmlFromFile = usePracticeStore(
    (s) => s.importMusicXmlFromFile,
  );
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
  // `entry` is diagnostic only (#336): Tauri's drag-drop and the file picker
  // may deliver differently in the real shell, so breadcrumbs record which
  // path each import took.
  const handleFile = async (file: File, entry: ImportEntryPath) => {
    importSeq.current += 1;
    const seq = importSeq.current;
    const isCurrentImport = () => importSeq.current === seq;
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
      // #336 diagnostics: which exit the feedback lifecycle takes. "error"
      // until the success path proves otherwise; a stale import whose
      // writes were silenced reports "superseded" instead.
      let outcome: ImportOutcome = "error";
      try {
        setStatus(`Transcribing ${file.name}…`);
        unlisten = await listen<ImportProgress>("import-progress", (event) => {
          setProgress(event.payload);
        });
        const buffer = await file.arrayBuffer();
        const bytes = Array.from(new Uint8Array(buffer));
        // #336: the loading message must not DEPEND on event delivery — the
        // VA saw imports finish without it ever painting. We know we're
        // transcribing the moment the call starts, so seed the indicator
        // (the backend's own beats are fixed constants; events refine it).
        const shownAt = performance.now();
        setProgress({ stage: "transcribing", pct: 45 });
        sendBreadcrumb(importSeededBreadcrumb(seq, "transcribing", entry, ext));
        scheduleImportFirstFrameProbe(seq, rootRef.current, sendBreadcrumb);
        const result = await importAudioFromFile(file.name, bytes);
        // Success keeps the indicator up for a perceivable minimum (#336);
        // a failure below skips straight to the error. A newer import may
        // have started during the hold — then this one stays silent.
        await holdImportFeedback(shownAt, importFeedbackMinMs);
        sendBreadcrumb(importHoldBreadcrumb(seq, performance.now() - shownAt));
        outcome = "success";
        if (isCurrentImport()) {
          // AC2 (#337 S1): transcription is the beta tier — say so at import
          // time, every time, not only when quality looks off.
          setStatus(
            `Imported "${result.entry.title}" — transcribed from audio (beta): check the notes look right.`,
          );
          // Only surface the banner when something looks off — never nag.
          // A clean mono take stays silent (#331 AC1's negative half).
          if (result.verdict !== "mono" || result.low_confidence) {
            setQuality(result);
          }
        }
      } catch (err) {
        if (isCurrentImport()) {
          setStatus(null);
          setError(`${err instanceof Error ? err.message : err}`);
        }
      } finally {
        unlisten?.();
        const superseded = !isCurrentImport();
        if (!superseded) setProgress(null);
        sendBreadcrumb(importClearedBreadcrumb(seq, outcome, superseded));
      }
      return;
    }

    if (PDF_EXTS.includes(ext)) {
      // PDF → on-device OMR → MusicXML, then the SAME "which part?" picker and
      // import path as MusicXML (OMR is just another front-end producing it).
      let unlisten: (() => void) | undefined;
      let outcome: ImportOutcome = "error";
      try {
        setStatus(`Reading ${file.name}…`);
        unlisten = await listen<ImportProgress>("import-progress", (event) => {
          setProgress(event.payload);
        });
        const buffer = await file.arrayBuffer();
        const bytes = Array.from(new Uint8Array(buffer));
        // #336: same event-independent seed as the audio path.
        const shownAt = performance.now();
        setProgress({ stage: "reading-notes", pct: 55 });
        sendBreadcrumb(
          importSeededBreadcrumb(seq, "reading-notes", entry, ext),
        );
        scheduleImportFirstFrameProbe(seq, rootRef.current, sendBreadcrumb);
        const recognized = await recognizePdfFromFile(file.name, bytes);
        // Same perceivable minimum as the audio path (#336); same
        // newer-import-wins rule after it.
        await holdImportFeedback(shownAt, importFeedbackMinMs);
        sendBreadcrumb(importHoldBreadcrumb(seq, performance.now() - shownAt));
        outcome = "success";
        if (isCurrentImport()) {
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
        }
      } catch (err) {
        if (isCurrentImport()) {
          setStatus(null);
          setError(`${err instanceof Error ? err.message : err}`);
        }
      } finally {
        unlisten?.();
        const superseded = !isCurrentImport();
        if (!superseded) setProgress(null);
        sendBreadcrumb(importClearedBreadcrumb(seq, outcome, superseded));
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
    await handleFile(files[0], "drag-drop");
  };

  const handleBrowse = () => {
    fileInputRef.current?.click();
  };

  const handleFileInput = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.currentTarget.files;
    if (!files || files.length === 0) return;
    await handleFile(files[0], "file-picker");
  };

  // The honesty gate (#331): the verdict is classified in the Rust core;
  // this only picks the words. Full mix gets the plain-spoken version.
  const qualityMessage =
    quality?.verdict === "full_mix"
      ? "This sounds like a full mix — the notes may be approximate. Practicing works best with one instrument's line at a time."
      : quality?.verdict === "borderline"
        ? "This recording sounds polyphonic — basic-pitch works best on a single instrument line, so the transcription may be rough. You can re-record or drop a different file anytime."
        : "This transcription may be approximate — try a closer, single-instrument recording for a cleaner result.";
  const honestyCounts = quality
    ? `Caught ${quality.note_count} note${quality.note_count === 1 ? "" : "s"}, ${quality.uncertain_count} uncertain.`
    : null;

  return (
    <div
      ref={rootRef}
      className="rounded-lg border-2 border-dashed border-gray-600 bg-gray-800 p-8"
    >
      <div
        data-testid="score-drop-zone"
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
          {isDragging
            ? "Drop your score here"
            : "Drag a score or recording here"}
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
                  if (partImportInFlight.current) return;
                  partImportInFlight.current = true;
                  try {
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
                  } finally {
                    partImportInFlight.current = false;
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
          <span>
            {qualityMessage}
            <span className="mt-1 block text-amber-300/80 text-xs">
              {honestyCounts}
            </span>
          </span>
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
