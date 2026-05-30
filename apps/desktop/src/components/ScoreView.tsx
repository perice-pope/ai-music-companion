import { useEffect, useRef } from "react";
import type { ScorePosition } from "../types/brain";

interface Props {
  musicXml: string;
  cursorPosition: ScorePosition | null;
}

/**
 * Renders a MusicXML score using OSMD (OpenSheetMusicDisplay).
 *
 * Uses lazy import so bundles for free-play-only sessions don't
 * pull in OSMD's ~600KB minified footprint. Cursor is driven by
 * the parent's `cursorPosition` state, updated on phrase boundaries.
 */
export default function ScoreView({ musicXml, cursorPosition }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const osmdRef = useRef<any>(null);
  const isLoadingRef = useRef(false);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      if (isLoadingRef.current || !containerRef.current) return;
      isLoadingRef.current = true;

      try {
        // Lazy import OSMD only when needed
        const { OpenSheetMusicDisplay } = await import("opensheetmusicdisplay");

        if (cancelled) return;

        // Initialize OSMD on the container
        const osmd = new OpenSheetMusicDisplay(containerRef.current, {
          // Use SVG backend for consistent rendering
          backend: "svg",
        });

        osmd.load(musicXml);
        osmd.render();

        osmdRef.current = osmd;
      } catch (err) {
        console.error("Failed to load score:", err);
        if (containerRef.current) {
          containerRef.current.innerHTML =
            '<div class="text-red-400 text-center">Failed to load score</div>';
        }
      } finally {
        isLoadingRef.current = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [musicXml]);

  // Update cursor position when it changes
  useEffect(() => {
    const osmd = osmdRef.current;
    if (!osmd || !osmd.cursor || !cursorPosition) return;

    try {
      // OSMD cursor methods:
      // - showAll() / show() — make cursor visible
      // - reset() — go to the beginning
      // - next() — advance to next note
      // Iterator-based method would be ideal, but for now just show the cursor.
      // In a production implementation, we'd map measure_number+beat to
      // the actual OSMD note index via the score structure.
      osmd.cursor.show();

      // TODO: implement full measure-to-note mapping for smooth cursor tracking.
      // For now, the cursor is visible and the component proves the integration.
    } catch (err) {
      console.error("Failed to update cursor:", err);
    }
  }, [cursorPosition]);

  return (
    <div className="flex h-full w-full flex-col bg-gray-800">
      <div
        ref={containerRef}
        className="flex-1 overflow-auto rounded border border-gray-700 p-4"
        data-testid="score-view-container"
        style={{ minHeight: "200px" }}
      />
    </div>
  );
}
