import { useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";
import CellStaff from "./CellStaff";
import type {
  StarterItem,
  StarterChordKind,
  StarterScaleKind,
  StarterEnclosureStyle,
} from "../types/brain";

/**
 * #419 S1 — Openers: "start with something in your hands."
 *
 * The RV builder's front door, reborn: compose an opener from the item
 * bank, watch it render live on the RV dot staff, and Begin — the opener
 * becomes the session's exploration (rowed through 12 keys) and hands off
 * into normal listening when you're done. S1 ships the first two bank
 * entries; the rest are visible but resting (the roadmap, honestly).
 *
 * #417 rule 0 applies from birth: the preview updates in place; nothing
 * here blinks, slides, or vanishes.
 */

/**
 * Major-scale degree labels for the note buttons. Taps send SEMANTIC
 * degrees over the wire — the degree→semitone table lives in
 * `brain::starter` only (review MF2: no pitch math in the frontend).
 */
const DEGREES = [1, 2, 3, 4, 5, 6, 7, 8] as const;

/** The classic openers, ready-made. */
const SEQUENCE_PRESETS: { label: string; degrees: number[] }[] = [
  { label: "1-2-3-5", degrees: [1, 2, 3, 5] },
  { label: "1-3-5-8", degrees: [1, 3, 5, 8] },
  { label: "5-4-3-2-1", degrees: [5, 4, 3, 2, 1] },
];

/** #419 S2a: the live bank. Labels read musically; values are the wire. */
const INTERVALS: { label: string; number: number }[] = [
  { label: "2nd", number: 2 },
  { label: "3rd", number: 3 },
  { label: "4th", number: 4 },
  { label: "5th", number: 5 },
  { label: "6th", number: 6 },
  { label: "7th", number: 7 },
  { label: "8ve", number: 8 },
];
const CHORDS: { label: string; kind: StarterChordKind }[] = [
  { label: "maj", kind: "major_triad" },
  { label: "min", kind: "minor_triad" },
  { label: "7", kind: "dominant_seventh" },
  { label: "maj7", kind: "major_seventh" },
  { label: "min7", kind: "minor_seventh" },
];
const SCALES: { label: string; kind: StarterScaleKind }[] = [
  { label: "major", kind: "major" },
  { label: "minor", kind: "natural_minor" },
  { label: "maj pent", kind: "major_pentatonic" },
  { label: "min pent", kind: "minor_pentatonic" },
  { label: "blues", kind: "blues" },
  { label: "dorian", kind: "dorian" },
  { label: "mixo", kind: "mixolydian" },
];
const ENCLOSURES: { label: string; style: StarterEnclosureStyle }[] = [
  { label: "enclose ↓↑", style: "one_down_one_up" },
  { label: "enclose ↑↓", style: "one_up_one_down" },
];

/** Still resting: direction rides S2b, My patterns rides S3. */
const COMING_SOON = ["Pattern directions", "My patterns"];

const CHORD_LABELS: Record<StarterChordKind, string> = {
  major_triad: "maj",
  minor_triad: "min",
  dominant_seventh: "7",
  major_seventh: "maj7",
  minor_seventh: "min7",
};
const SCALE_LABELS: Record<StarterScaleKind, string> = {
  major: "major",
  natural_minor: "minor",
  major_pentatonic: "maj pent",
  minor_pentatonic: "min pent",
  blues: "blues",
  dorian: "dorian",
  mixolydian: "mixo",
};

/** Human label for an added item chip — derived from the semantic wire
 * shape, never from offsets (offsets are the backend's business). */
function itemLabel(item: StarterItem): string {
  switch (item.type) {
    case "notes":
      return `notes ×${item.offsets.length}`;
    case "note_sequence":
      return item.degrees.length === 1
        ? `note ${item.degrees[0]}`
        : item.degrees.join("-");
    case "interval":
      return item.number === 8 ? "8ve" : `${item.number}th`;
    case "chord":
      return CHORD_LABELS[item.kind];
    case "scale":
      return SCALE_LABELS[item.kind];
    case "enclosure":
      return item.style === "one_down_one_up" ? "enclose ↓↑" : "enclose ↑↓";
  }
}

/** #419 S2a: parse the custom entry — digits 1-9 separated by spaces,
 * dashes, or commas — into degrees. Returns null on junk (the panel shows
 * a calm client-side notice; VALIDITY of degrees stays the backend's).
 */
export function parseCustomSequence(raw: string): number[] | null {
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    return null;
  }
  const parts = trimmed.split(/[\s,-]+/).filter((p) => p.length > 0);
  const degrees = parts.map((p) => Number(p));
  if (degrees.some((d) => !Number.isInteger(d) || d < 0 || d > 99)) {
    return null;
  }
  return degrees;
}

export default function OpenersPanel() {
  const openerItems = usePracticeStore((s) => s.openerItems);
  const openerPreview = usePracticeStore((s) => s.openerPreview);
  const openerNotice = usePracticeStore((s) => s.openerNotice);
  const addOpenerItem = usePracticeStore((s) => s.addOpenerItem);
  const removeOpenerItem = usePracticeStore((s) => s.removeOpenerItem);
  const beginOpener = usePracticeStore((s) => s.beginOpener);
  const [open, setOpen] = useState(false);
  const [customSeq, setCustomSeq] = useState("");
  const [customNotice, setCustomNotice] = useState<string | null>(null);

  const addCustom = () => {
    const degrees = parseCustomSequence(customSeq);
    if (!degrees) {
      // Junk never goes over the wire — but real out-of-range degrees DO
      // (the backend's refusals name them better than we could).
      setCustomNotice("numbers only — like 1 5 3 2");
      return;
    }
    setCustomNotice(null);
    setCustomSeq("");
    void addOpenerItem({ type: "note_sequence", degrees });
  };

  if (!open) {
    return (
      <button
        type="button"
        data-testid="openers-toggle"
        onClick={() => setOpen(true)}
        className="rounded-full border border-teal-700 bg-teal-900/30 px-4 py-1.5 text-sm text-teal-200 hover:bg-teal-900/50"
      >
        🎬 Openers — start with something in your hands
      </button>
    );
  }

  return (
    <div
      data-testid="openers-panel"
      className="w-full max-w-md rounded-lg border border-teal-800 bg-teal-950/30 p-4 text-left"
    >
      <div className="flex items-center justify-between">
        <p className="text-sm font-semibold text-teal-200">🎬 Openers</p>
        <button
          type="button"
          data-testid="openers-close"
          onClick={() => setOpen(false)}
          className="text-sm text-teal-400/70 hover:text-teal-200"
          aria-label="Close openers"
        >
          ×
        </button>
      </div>

      {/* The bank — S1's two live entries. */}
      <p className="mt-3 text-xs uppercase tracking-wider text-teal-400/80">
        Notes
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {DEGREES.map((d) => (
          <button
            key={d}
            type="button"
            data-testid={`opener-note-${d}`}
            onClick={() =>
              void addOpenerItem({ type: "note_sequence", degrees: [d] })
            }
            className="h-8 w-8 rounded-md bg-teal-800/60 text-sm font-semibold text-teal-100 hover:bg-teal-700"
          >
            {d}
          </button>
        ))}
      </div>

      <p className="mt-3 text-xs uppercase tracking-wider text-teal-400/80">
        Note sequence
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {SEQUENCE_PRESETS.map((p) => (
          <button
            key={p.label}
            type="button"
            data-testid={`opener-seq-${p.label}`}
            onClick={() =>
              void addOpenerItem({ type: "note_sequence", degrees: p.degrees })
            }
            className="rounded-md bg-teal-800/60 px-2.5 py-1 text-sm text-teal-100 hover:bg-teal-700"
          >
            {p.label}
          </button>
        ))}
      </div>

      {/* #419 S2a: custom sequence — parsed client-side into the SAME
          note_sequence wire shape; degree validity stays the backend's. */}
      <div className="mt-2 flex gap-1.5">
        <input
          type="text"
          data-testid="opener-custom-input"
          value={customSeq}
          onChange={(e) => setCustomSeq(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") addCustom();
          }}
          placeholder="your own: 1 5 3 2"
          className="w-40 rounded-md border border-teal-800 bg-teal-950/60 px-2 py-1 text-sm text-teal-100 placeholder-teal-700"
        />
        <button
          type="button"
          data-testid="opener-custom-add"
          onClick={addCustom}
          className="rounded-md bg-teal-800/60 px-2.5 py-1 text-sm text-teal-100 hover:bg-teal-700"
        >
          Add
        </button>
      </div>

      <p className="mt-3 text-xs uppercase tracking-wider text-teal-400/80">
        Intervals
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {INTERVALS.map((iv) => (
          <button
            key={iv.number}
            type="button"
            data-testid={`opener-interval-${iv.number}`}
            onClick={() =>
              void addOpenerItem({ type: "interval", number: iv.number })
            }
            className="rounded-md bg-teal-800/60 px-2.5 py-1 text-sm text-teal-100 hover:bg-teal-700"
          >
            {iv.label}
          </button>
        ))}
      </div>

      <p className="mt-3 text-xs uppercase tracking-wider text-teal-400/80">
        Chords
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {CHORDS.map((c) => (
          <button
            key={c.kind}
            type="button"
            data-testid={`opener-chord-${c.kind}`}
            onClick={() => void addOpenerItem({ type: "chord", kind: c.kind })}
            className="rounded-md bg-teal-800/60 px-2.5 py-1 text-sm text-teal-100 hover:bg-teal-700"
          >
            {c.label}
          </button>
        ))}
      </div>

      <p className="mt-3 text-xs uppercase tracking-wider text-teal-400/80">
        Scales
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {SCALES.map((sc) => (
          <button
            key={sc.kind}
            type="button"
            data-testid={`opener-scale-${sc.kind}`}
            onClick={() => void addOpenerItem({ type: "scale", kind: sc.kind })}
            className="rounded-md bg-teal-800/60 px-2.5 py-1 text-sm text-teal-100 hover:bg-teal-700"
          >
            {sc.label}
          </button>
        ))}
      </div>

      <p className="mt-3 text-xs uppercase tracking-wider text-teal-400/80">
        Enclosures
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {ENCLOSURES.map((en) => (
          <button
            key={en.style}
            type="button"
            data-testid={`opener-enclosure-${en.style}`}
            onClick={() =>
              void addOpenerItem({ type: "enclosure", style: en.style })
            }
            className="rounded-md bg-teal-800/60 px-2.5 py-1 text-sm text-teal-100 hover:bg-teal-700"
          >
            {en.label}
          </button>
        ))}
      </div>

      {/* The rest of the bank, resting — the builder's shape, honestly. */}
      <div className="mt-2 flex flex-wrap gap-1.5">
        {COMING_SOON.map((name) => (
          <span
            key={name}
            className="rounded-md border border-teal-900 px-2 py-0.5 text-xs text-teal-700"
            title="coming soon"
          >
            {name}
          </span>
        ))}
      </div>

      {/* What's been added. */}
      {openerItems.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-1.5" data-testid="opener-chips">
          {openerItems.map((item, i) => (
            <button
              key={`${itemLabel(item)}-${i}`}
              type="button"
              data-testid={`opener-chip-${i}`}
              onClick={() => void removeOpenerItem(i)}
              title="Remove"
              className="rounded-full bg-teal-800 px-2.5 py-0.5 text-xs text-teal-100 hover:bg-red-900/60"
            >
              {itemLabel(item)} ×
            </button>
          ))}
        </div>
      )}

      {/* The live staff preview — the credibility moment. */}
      {openerPreview && (
        <div className="mt-3" data-testid="opener-preview">
          <CellStaff staff={openerPreview.staff} />
        </div>
      )}
      {customNotice && (
        <p
          className="mt-2 text-sm text-amber-300"
          data-testid="opener-custom-notice"
        >
          {customNotice}
        </p>
      )}
      {openerNotice && (
        <p className="mt-2 text-sm text-amber-300" data-testid="opener-notice">
          {openerNotice}
        </p>
      )}

      <button
        type="button"
        data-testid="opener-begin"
        disabled={openerItems.length === 0}
        onClick={() => void beginOpener()}
        className="mt-4 w-full rounded-md bg-teal-600 px-4 py-2 text-sm font-semibold text-white hover:bg-teal-500 disabled:opacity-40"
      >
        Begin
      </button>
    </div>
  );
}
