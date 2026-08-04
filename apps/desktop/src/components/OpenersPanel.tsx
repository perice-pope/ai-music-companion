import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
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
 * #471 pt 3 — reborn RV-simple: the default face is twelve note buttons,
 * the live preview, and Begin (the two-button philosophy — pick notes,
 * go). The whole grown bank (sequences, intervals, chords, scales,
 * enclosures, directions, custom entry) folds behind ONE collapsed
 * "More options" disclosure; My Patterns / Recipes / Yesterday stay
 * visible — they ARE the one-tap simplicity.
 *
 * #417 rule 0 applies from birth: the preview updates in place; nothing
 * here blinks, slides, or vanishes.
 */

/**
 * #471 pt 3 — the chromatic picker: one button per pitch class relative
 * to the root ("12-tone rows are the basis of RV"). Button k is k
 * semitones above the root; labels read musically. Taps build ONE
 * `Notes{offsets}` item, RE-BASED to the first tap (offsets[i] =
 * tap[i] − tap[0]) so the first offset is always 0 — the documented
 * `Notes` convention ("offsets from the cell's first note") — and
 * lower buttons go legally negative. The alternative (send k raw) was
 * rejected: the same tapped shape would sound transposed depending on
 * which button led. The first note you tap is where the row starts.
 */
const PITCH_CLASSES = [
  "1",
  "♭2",
  "2",
  "♭3",
  "3",
  "4",
  "♭5",
  "5",
  "♭6",
  "6",
  "♭7",
  "7",
] as const;

/**
 * Major-scale degree labels for the note buttons (More options). Taps
 * send SEMANTIC degrees over the wire — the degree→semitone table lives
 * in `brain::starter` only (review MF2: no pitch math in the frontend).
 * 1..=12 mirrors the backend's vocabulary: 8 = the octave, 9..=12 the
 * compound extension (#471 pt 3). Degrees are diatonic; the chromatic
 * gesture is the picker above, on the `notes` wire.
 */
const DEGREES = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] as const;

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

type OpenerDirection = "forward" | "reversed" | "varied";

/** #419 S2b: recipe-level directions — exclusive chips, forward default. */
const DIRECTIONS: { label: string; value: OpenerDirection }[] = [
  { label: "forward", value: "forward" },
  { label: "reversed", value: "reversed" },
  { label: "varied", value: "varied" },
];

/** #419 S3: a pattern your hands actually played, from the exercise log. */
interface MyPattern {
  label: string;
  offsets: number[];
  times_practiced: number;
  last_tonic: number;
}

/** #419 S4: a saved recipe row. Mirrors `commands::RecipeDto`. */
interface SavedRecipe {
  id: number;
  name: string;
  items: StarterItem[];
  direction: OpenerDirection;
}

/** #419 S4: yesterday's opener. Mirrors `commands::LastOpenerDto`. */
interface LastOpener {
  label: string;
  tonic: number;
}

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
      // Review MF4: real ordinals ("2th" is not music). The bank table is
      // the single source for these labels.
      return (
        INTERVALS.find((iv) => iv.number === item.number)?.label ??
        `${item.number}th`
      );
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
  // The 0..99 bound is load-bearing: it keeps every crossing value in u8
  // range, so the backend always answers with its calm NAMED refusal and
  // never a raw serde overflow error.
  if (degrees.some((d) => !Number.isInteger(d) || d < 0 || d > 99)) {
    return null;
  }
  return degrees;
}

/** Every bank row wears the same chip. */
const BANK_CHIP =
  "rounded-md bg-indigo-800/60 px-2.5 py-1 text-sm text-indigo-100 hover:bg-indigo-700";

/** A bank row: uppercase heading over a wrapping chip strip. */
function BankSection({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <>
      <p className="mt-3 text-xs uppercase tracking-wider text-indigo-300/70">
        {title}
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">{children}</div>
    </>
  );
}

/**
 * The folded bank behind "More options" (#471 pt 3): the sub-banks, the
 * custom entry, and the direction chips. Presentation only — every tap
 * speaks the same semantic wire shapes through the panel's handlers, and
 * the custom entry's state stays with the panel so its notice keeps
 * rendering beside the other panel notices.
 *
 * Must stay module-level: defined inside OpenersPanel it would be a new
 * component type each render, remounting the custom input (and dropping
 * its focus) on every keystroke.
 */
function OpenerBank({
  onAdd,
  direction,
  onDirection,
  customSeq,
  onCustomSeq,
  onAddCustom,
}: {
  onAdd: (item: StarterItem) => Promise<void>;
  direction: OpenerDirection;
  onDirection: (d: OpenerDirection) => void;
  customSeq: string;
  onCustomSeq: (value: string) => void;
  onAddCustom: () => void;
}) {
  return (
    <div data-testid="opener-more">
      {/* The bank — S1's two live entries. */}
      <BankSection title="Notes">
        {DEGREES.map((d) => (
          <button
            key={d}
            type="button"
            data-testid={`opener-note-${d}`}
            onClick={() => void onAdd({ type: "note_sequence", degrees: [d] })}
            className="h-8 w-8 rounded-md bg-indigo-800/60 text-sm font-semibold text-indigo-100 hover:bg-indigo-700"
          >
            {d}
          </button>
        ))}
      </BankSection>

      <BankSection title="Note sequence">
        {SEQUENCE_PRESETS.map((p) => (
          <button
            key={p.label}
            type="button"
            data-testid={`opener-seq-${p.label}`}
            onClick={() =>
              void onAdd({ type: "note_sequence", degrees: p.degrees })
            }
            className={BANK_CHIP}
          >
            {p.label}
          </button>
        ))}
      </BankSection>

      {/* #419 S2a: custom sequence — parsed client-side into the SAME
      note_sequence wire shape; degree validity stays the backend's. */}
      <div className="mt-2 flex gap-1.5">
        <input
          type="text"
          data-testid="opener-custom-input"
          value={customSeq}
          onChange={(e) => onCustomSeq(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onAddCustom();
          }}
          placeholder="your own: 1 5 3 2"
          className="w-40 rounded-md border border-indigo-800 bg-indigo-950/60 px-2 py-1 text-sm text-indigo-100 placeholder-gray-500"
        />
        <button
          type="button"
          data-testid="opener-custom-add"
          onClick={onAddCustom}
          className={BANK_CHIP}
        >
          Add
        </button>
      </div>

      <BankSection title="Intervals">
        {INTERVALS.map((iv) => (
          <button
            key={iv.number}
            type="button"
            data-testid={`opener-interval-${iv.number}`}
            onClick={() => void onAdd({ type: "interval", number: iv.number })}
            className={BANK_CHIP}
          >
            {iv.label}
          </button>
        ))}
      </BankSection>

      <BankSection title="Chords">
        {CHORDS.map((c) => (
          <button
            key={c.kind}
            type="button"
            data-testid={`opener-chord-${c.kind}`}
            onClick={() => void onAdd({ type: "chord", kind: c.kind })}
            className={BANK_CHIP}
          >
            {c.label}
          </button>
        ))}
      </BankSection>

      <BankSection title="Scales">
        {SCALES.map((sc) => (
          <button
            key={sc.kind}
            type="button"
            data-testid={`opener-scale-${sc.kind}`}
            onClick={() => void onAdd({ type: "scale", kind: sc.kind })}
            className={BANK_CHIP}
          >
            {sc.label}
          </button>
        ))}
      </BankSection>

      <BankSection title="Enclosures">
        {ENCLOSURES.map((en) => (
          <button
            key={en.style}
            type="button"
            data-testid={`opener-enclosure-${en.style}`}
            onClick={() => void onAdd({ type: "enclosure", style: en.style })}
            className={BANK_CHIP}
          >
            {en.label}
          </button>
        ))}
      </BankSection>

      <BankSection title="Pattern direction">
        {DIRECTIONS.map((d) => (
          <button
            key={d.value}
            type="button"
            data-testid={`opener-direction-${d.value}`}
            aria-pressed={direction === d.value}
            onClick={() => onDirection(d.value)}
            className={`rounded-md px-2.5 py-1 text-sm ${
              direction === d.value
                ? "bg-indigo-600 font-semibold text-white"
                : "bg-indigo-800/60 text-indigo-100 hover:bg-indigo-700"
            }`}
          >
            {d.label}
          </button>
        ))}
      </BankSection>
    </div>
  );
}

/**
 * #471 pt 3 — the RV-simple face: one button per pitch class, tap order =
 * note order, order badges on the lit buttons. Presentation only — the
 * tap→item compilation (and the picks state it rides on) stays with the
 * panel, which owns the compiled item's identity inside openerItems.
 */
function ChromaticPicker({
  picks,
  onToggle,
}: {
  picks: number[];
  onToggle: (k: number) => void;
}) {
  return (
    <>
      <p className="mt-3 text-xs uppercase tracking-wider text-indigo-300/70">
        Pick your notes · tap in order, up to 12
      </p>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {PITCH_CLASSES.map((label, k) => {
          const pos = picks.indexOf(k);
          return (
            <button
              key={label}
              type="button"
              data-testid={`opener-pc-${k}`}
              aria-pressed={pos >= 0}
              onClick={() => onToggle(k)}
              className={`relative h-9 w-9 rounded-md text-sm font-semibold ${
                pos >= 0
                  ? "bg-indigo-600 text-white"
                  : "bg-indigo-800/60 text-indigo-100 hover:bg-indigo-700"
              }`}
            >
              {label}
              {pos >= 0 && (
                <span
                  // Visual order badge only — without aria-hidden, screen
                  // readers read "♭3 2" as if both were the note's name.
                  aria-hidden="true"
                  className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-indigo-300 text-[10px] font-bold text-indigo-950"
                >
                  {pos + 1}
                </span>
              )}
            </button>
          );
        })}
      </div>
    </>
  );
}

/** #419 S3: the one-tap strip of patterns your hands actually played.
 * Presentation only — the fetch (and its calm-empty degradation) stays
 * with the panel; taps speak the same Notes wire as the picker. */
function MyPatternsStrip({
  patterns,
  onAdd,
}: {
  patterns: MyPattern[];
  onAdd: (item: StarterItem) => Promise<void>;
}) {
  return (
    <>
      <p className="mt-3 text-xs uppercase tracking-wider text-indigo-300/70">
        My patterns
      </p>
      {patterns.length === 0 ? (
        <p
          className="mt-1 text-xs text-gray-500"
          data-testid="my-patterns-empty"
        >
          play and lift a few things first — your patterns appear here
        </p>
      ) : (
        <div className="mt-1 flex flex-wrap gap-1.5" data-testid="my-patterns">
          {patterns.map((p, i) => (
            <button
              key={`${p.label}-${i}`}
              type="button"
              data-testid={`opener-my-pattern-${i}`}
              onClick={() => void onAdd({ type: "notes", offsets: p.offsets })}
              className="rounded-md bg-indigo-800/60 px-2.5 py-1 text-sm text-indigo-100 hover:bg-indigo-700"
              title={p.label}
            >
              {p.label}
            </button>
          ))}
        </div>
      )}
    </>
  );
}

/**
 * #419 S4 — the recall surfaces: yesterday's opener, the saved-recipe
 * strip, and the save row. Presentation only — fetch, save, and forget
 * stay with the panel so the strip's data and notices share the panel's
 * lifecycle.
 *
 * Must stay module-level: defined inside OpenersPanel it would be a new
 * component type each render, remounting the name input (and dropping
 * its focus) on every keystroke.
 */
function RecipesSection({
  lastOpener,
  recipes,
  recipeName,
  recipeNotice,
  showSave,
  onRecipeName,
  onSave,
  onForget,
  onApply,
  onRecall,
}: {
  lastOpener: LastOpener | null;
  recipes: SavedRecipe[];
  recipeName: string;
  recipeNotice: string | null;
  showSave: boolean;
  onRecipeName: (value: string) => void;
  onSave: () => void;
  onForget: (id: number) => void;
  onApply: (recipe: SavedRecipe) => void;
  onRecall: () => void;
}) {
  return (
    <>
      <p className="mt-3 text-xs uppercase tracking-wider text-indigo-300/70">
        Recipes
      </p>
      {lastOpener ? (
        <button
          type="button"
          data-testid="opener-yesterday"
          onClick={onRecall}
          className="mt-1 rounded-md bg-indigo-800/60 px-2.5 py-1 text-sm text-indigo-100 hover:bg-indigo-700"
          title="replay it exactly — same notes, same journey"
        >
          ⟲ yesterday: {lastOpener.label}
        </button>
      ) : (
        <p
          className="mt-1 text-xs text-gray-500"
          data-testid="opener-yesterday-empty"
        >
          begin an opener and it&apos;ll be waiting here tomorrow
        </p>
      )}
      {recipes.length === 0 ? (
        <p
          className="mt-1 text-xs text-gray-500"
          data-testid="opener-recipes-empty"
        >
          name and save a builder you like — it&apos;ll live here
        </p>
      ) : (
        <div
          className="mt-1 flex flex-wrap gap-1.5"
          data-testid="opener-recipes"
        >
          {recipes.map((r) => (
            <span
              key={r.id}
              className="inline-flex items-center gap-1 rounded-md bg-indigo-800/60 pr-1"
            >
              <button
                type="button"
                data-testid={`opener-recipe-${r.id}`}
                onClick={() => onApply(r)}
                className="px-2.5 py-1 text-sm text-indigo-100 hover:text-white"
                title={`${r.items.length} item${r.items.length === 1 ? "" : "s"}, ${r.direction}`}
              >
                {r.name}
              </button>
              <button
                type="button"
                data-testid={`opener-recipe-delete-${r.id}`}
                onClick={() => onForget(r.id)}
                aria-label={`Forget ${r.name}`}
                className="inline-flex h-4 w-4 items-center justify-center rounded-full text-xs text-indigo-400/70 hover:text-white"
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
      {showSave && (
        <div className="mt-2 flex gap-1.5">
          <input
            data-testid="opener-recipe-name"
            value={recipeName}
            onChange={(e) => onRecipeName(e.target.value)}
            placeholder="name this recipe"
            className="w-40 rounded-md border border-indigo-800 bg-indigo-950/60 px-2 py-1 text-sm text-indigo-100 placeholder:text-gray-500"
          />
          <button
            type="button"
            data-testid="opener-recipe-save"
            onClick={onSave}
            disabled={recipeName.trim().length === 0}
            className="rounded-md bg-indigo-600/80 px-2.5 py-1 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-40"
          >
            Save
          </button>
        </div>
      )}
      {recipeNotice && (
        <p
          className="mt-1 text-xs text-amber-300"
          data-testid="opener-recipe-notice"
        >
          {recipeNotice}
        </p>
      )}
    </>
  );
}

export default function OpenersPanel() {
  const openerItems = usePracticeStore((s) => s.openerItems);
  const openerPreview = usePracticeStore((s) => s.openerPreview);
  const openerNotice = usePracticeStore((s) => s.openerNotice);
  const addOpenerItem = usePracticeStore((s) => s.addOpenerItem);
  const removeOpenerItem = usePracticeStore((s) => s.removeOpenerItem);
  const beginOpener = usePracticeStore((s) => s.beginOpener);
  const openerDirection = usePracticeStore((s) => s.openerDirection);
  const setOpenerDirection = usePracticeStore((s) => s.setOpenerDirection);
  const applyOpenerRecipe = usePracticeStore((s) => s.applyOpenerRecipe);
  const beginOpenerRecall = usePracticeStore((s) => s.beginOpenerRecall);
  const [open, setOpen] = useState(false);
  // #471 pt 3: the folded bank — collapsed by default (RV-simple face).
  const [more, setMore] = useState(false);
  // #471 pt 3: the chromatic picker's taps, in tap order (button values
  // 0..11). The compiled item lives in openerItems like any other; this
  // is only which buttons are lit and in what order.
  const [picks, setPicks] = useState<number[]>([]);
  const pickerItemRef = useRef<StarterItem | null>(null);
  const [myPatterns, setMyPatterns] = useState<MyPattern[]>([]);
  const [recipes, setRecipes] = useState<SavedRecipe[]>([]);
  const [lastOpener, setLastOpener] = useState<LastOpener | null>(null);
  const [recipeName, setRecipeName] = useState("");
  const [recipeNotice, setRecipeNotice] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    // #419 S3: fetched when the panel opens — a pattern earned
    // mid-session appears next open. Failures read as an empty list;
    // the empty state is honest either way.
    let cancelled = false;
    void invoke<MyPattern[]>("my_patterns")
      .then((patterns) => {
        if (!cancelled) {
          // A malformed response degrades to the honest empty state —
          // this panel never crashes over its own convenience feature.
          setMyPatterns(Array.isArray(patterns) ? patterns : []);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setMyPatterns([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  useEffect(() => {
    if (!open) {
      return;
    }
    // #419 S4: recall surfaces load with the panel — same calm rules as
    // My Patterns (failures and malformed answers read as honest empty).
    let cancelled = false;
    void invoke<SavedRecipe[]>("list_opener_recipes")
      .then((rows) => {
        if (!cancelled) {
          setRecipes(Array.isArray(rows) ? rows : []);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setRecipes([]);
        }
      });
    void invoke<LastOpener | null>("recall_last_opener")
      .then((last) => {
        if (!cancelled) {
          setLastOpener(last && typeof last.label === "string" ? last : null);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setLastOpener(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const saveRecipe = () => {
    void invoke<SavedRecipe>("save_opener_recipe", {
      name: recipeName,
      items: openerItems,
      direction: openerDirection,
    })
      .then((r) => {
        setRecipeName("");
        setRecipeNotice(null);
        setRecipes((prev) => [r, ...prev]);
      })
      .catch((e) => setRecipeNotice(String(e)));
  };

  const forgetRecipe = (id: number) => {
    void invoke("delete_opener_recipe", { id })
      .then(() => setRecipes((prev) => prev.filter((r) => r.id !== id)))
      .catch(() => {
        // The row being gone IS the requested state — nothing to say.
      });
  };
  const [customSeq, setCustomSeq] = useState("");
  const [customNotice, setCustomNotice] = useState<string | null>(null);

  // #471 pt 3: the picker's lit state follows the store — when its item
  // leaves openerItems (chip removed, Begin's reset, a recipe applied,
  // session end), the buttons go dark. No ghost-lit picker over an
  // empty builder.
  useEffect(() => {
    const current = pickerItemRef.current;
    if (current && !openerItems.includes(current)) {
      pickerItemRef.current = null;
      setPicks([]);
    }
  }, [openerItems]);

  // #471 pt 3: tap toggles a pitch class; tap order = note order. The
  // taps compile to ONE Notes item, re-based to the first tap (see
  // PITCH_CLASSES), which keeps its position among other items and
  // rides the existing set-items + pure-preview path.
  const togglePick = (k: number) => {
    const next = picks.includes(k)
      ? picks.filter((p) => p !== k)
      : [...picks, k];
    const item: StarterItem | null =
      next.length > 0
        ? { type: "notes", offsets: next.map((p) => p - next[0]) }
        : null;
    const idx = pickerItemRef.current
      ? openerItems.indexOf(pickerItemRef.current)
      : -1;
    const items = [...openerItems];
    if (idx >= 0) {
      if (item) {
        items[idx] = item;
      } else {
        items.splice(idx, 1);
      }
    } else if (item) {
      items.push(item);
    }
    pickerItemRef.current = item;
    setPicks(next);
    void applyOpenerRecipe(items, openerDirection);
  };

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
        className="rounded-full border border-indigo-800 bg-indigo-950/40 px-4 py-1.5 text-sm text-indigo-200 hover:bg-indigo-900/50"
      >
        🎬 Openers — start with something in your hands
      </button>
    );
  }

  return (
    <div
      data-testid="openers-panel"
      className="max-h-[70vh] w-full max-w-md overflow-y-auto rounded-lg border border-indigo-900 bg-indigo-950/30 p-4 text-left"
    >
      <div className="flex items-center justify-between">
        <p className="text-sm font-semibold text-indigo-200">🎬 Openers</p>
        <button
          type="button"
          data-testid="openers-close"
          onClick={() => setOpen(false)}
          className="text-sm text-indigo-400/70 hover:text-indigo-200"
          aria-label="Close openers"
        >
          ×
        </button>
      </div>

      {/* #471 pt 3 — the RV-simple face: twelve notes, tap in order. */}
      <ChromaticPicker picks={picks} onToggle={togglePick} />

      {/* Everything the builder grew lives here, folded (#471 pt 3). */}
      <button
        type="button"
        data-testid="opener-more-toggle"
        aria-expanded={more}
        onClick={() => setMore((m) => !m)}
        className="mt-3 text-xs uppercase tracking-wider text-indigo-300/70 hover:text-indigo-100"
      >
        More options {more ? "▴" : "▾"}
      </button>

      {more && (
        <OpenerBank
          onAdd={addOpenerItem}
          direction={openerDirection}
          onDirection={setOpenerDirection}
          customSeq={customSeq}
          onCustomSeq={setCustomSeq}
          onAddCustom={addCustom}
        />
      )}

      {/* #419 S3: the bank's last entry, live — YOUR patterns. */}
      <MyPatternsStrip patterns={myPatterns} onAdd={addOpenerItem} />

      {/* #419 S4: recall — yesterday's opener and the recipes you kept. */}
      <RecipesSection
        lastOpener={lastOpener}
        recipes={recipes}
        recipeName={recipeName}
        recipeNotice={recipeNotice}
        showSave={openerItems.length > 0}
        onRecipeName={setRecipeName}
        onSave={saveRecipe}
        onForget={forgetRecipe}
        onApply={(r) => void applyOpenerRecipe(r.items, r.direction)}
        onRecall={() => void beginOpenerRecall()}
      />

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
              className="rounded-full bg-indigo-800 px-2.5 py-0.5 text-xs text-indigo-100 hover:bg-red-900/60"
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
        className="sticky bottom-0 mt-4 w-full rounded-md bg-indigo-600 px-4 py-2 text-sm font-semibold text-white hover:bg-indigo-500 disabled:bg-indigo-950 disabled:text-indigo-400/60 disabled:hover:bg-indigo-950"
      >
        Begin
      </button>
    </div>
  );
}
