import { useRef, useState } from "react";

export default function ScoreDropZone() {
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleDrop = async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragging(false);
    setError(null);

    const files = e.dataTransfer.files;
    if (files.length === 0) return;

    const ext = files[0].name.split(".").pop()?.toLowerCase();
    const validExts = ["musicxml", "mxl", "xml", "mid"];

    if (!ext || !validExts.includes(ext)) {
      setError(
        `Unsupported format. Use .musicxml, .mxl, .xml, or .mid files.`
      );
      return;
    }

    try {
      // On Tauri, we need the file path. For now, we'll prompt the user
      // to use the "Choose file..." button. In a real implementation,
      // we'd use the filesystem API to get the path.
      // For now, just reject drop and show a friendly message.
      setError(
        `File drop not yet supported. Use "Choose file..." below.`
      );
    } catch (err) {
      setError(`Failed to load file: ${err}`);
    }
  };

  const handleBrowse = async () => {
    // In a Tauri app, we'd use open_dialog from tauri::api.
    // For now, trigger the file input.
    fileInputRef.current?.click();
  };

  const handleFileInput = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.currentTarget.files;
    if (!files || files.length === 0) return;

    setError(null);

    try {
      // Tauri can't directly access file paths from <input type="file">
      // for security reasons. This is where we'd use Tauri's file API.
      // For now, show an error.
      setError(
        `File input not yet integrated with Tauri. Use \`import_score\` command directly for testing.`
      );
    } catch (err) {
      setError(`Failed to load file: ${err}`);
    }
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
          Supports .musicxml, .mxl, .xml, and .mid files
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
            accept=".musicxml,.mxl,.xml,.mid"
            onChange={handleFileInput}
            className="hidden"
          />
        </div>
      </div>

      {error && (
        <div className="mt-4 rounded bg-red-900/20 border border-red-500 p-3 text-red-200 text-sm">
          {error}
        </div>
      )}
    </div>
  );
}
