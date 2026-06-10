import { useState } from "react";
import {
  EXPERIENCE_LEVELS,
  emptyTasteProfile,
  type ExperienceLevel,
  type TasteProfile,
} from "../types/brain";
import { useTasteStore } from "../stores/tasteStore";

/**
 * A calm onboarding wizard that captures the student's taste profile once:
 * genres, favourite artists, goals, and experience level. Writes the profile
 * locally via the `set_taste_profile` command (through `tasteStore`).
 *
 * On-brand: "coach, don't judge" — copy is warm, everything is skippable, and
 * nothing here grades the student. This is capture only; the profile is read by
 * the coaching layer elsewhere.
 */

/** A few starter genres to make the first step a one-tap experience. */
const GENRE_SUGGESTIONS = [
  "Classical",
  "Jazz",
  "Hip-hop",
  "Pop",
  "Film score",
  "Gospel",
  "Folk",
  "R&B",
  "Rock",
  "Latin",
];

/** Steps of the wizard, in order. */
const STEPS = ["genres", "artists", "goals", "experience"] as const;
type Step = (typeof STEPS)[number];

interface TasteOnboardingProps {
  /** Called once the profile has been saved (or skipped). */
  onComplete?: () => void;
}

export default function TasteOnboarding({ onComplete }: TasteOnboardingProps) {
  const save = useTasteStore((s) => s.save);
  const status = useTasteStore((s) => s.status);

  const [stepIndex, setStepIndex] = useState(0);
  const [draft, setDraft] = useState<TasteProfile>(emptyTasteProfile());
  const [artistInput, setArtistInput] = useState("");
  const [goalInput, setGoalInput] = useState("");

  const step: Step = STEPS[stepIndex];
  const isLast = stepIndex === STEPS.length - 1;

  const toggleGenre = (genre: string) => {
    setDraft((d) => ({
      ...d,
      genres: d.genres.includes(genre)
        ? d.genres.filter((g) => g !== genre)
        : [...d.genres, genre],
    }));
  };

  const addArtist = () => {
    const value = artistInput.trim();
    if (!value || draft.artists.includes(value)) return;
    setDraft((d) => ({ ...d, artists: [...d.artists, value] }));
    setArtistInput("");
  };

  const removeArtist = (artist: string) =>
    setDraft((d) => ({
      ...d,
      artists: d.artists.filter((a) => a !== artist),
    }));

  const addGoal = () => {
    const value = goalInput.trim();
    if (!value || draft.goals.includes(value)) return;
    setDraft((d) => ({ ...d, goals: [...d.goals, value] }));
    setGoalInput("");
  };

  const removeGoal = (goal: string) =>
    setDraft((d) => ({ ...d, goals: d.goals.filter((g) => g !== goal) }));

  const setExperience = (experience: ExperienceLevel) =>
    setDraft((d) => ({ ...d, experience }));

  const finish = async () => {
    await save(draft);
    onComplete?.();
  };

  const next = () => {
    if (isLast) {
      void finish();
    } else {
      setStepIndex((i) => i + 1);
    }
  };

  const back = () => setStepIndex((i) => Math.max(0, i - 1));

  const saving = status === "saving";

  return (
    <section
      aria-label="Get to know your music"
      className="mx-auto max-w-md rounded-lg bg-gray-800 p-6 text-gray-100"
    >
      <header className="mb-4">
        <h2 className="text-lg font-semibold text-white">
          Let&rsquo;s tune the coaching to you
        </h2>
        <p className="mt-1 text-sm text-gray-400">
          A few quick questions — no wrong answers. You can change these
          anytime.
        </p>
        <p className="mt-2 text-xs text-gray-500" aria-hidden="true">
          Step {stepIndex + 1} of {STEPS.length}
        </p>
      </header>

      {step === "genres" && (
        <fieldset>
          <legend className="mb-2 text-sm font-medium text-gray-200">
            What do you love listening to?
          </legend>
          <div className="flex flex-wrap gap-2">
            {GENRE_SUGGESTIONS.map((genre) => {
              const selected = draft.genres.includes(genre);
              return (
                <button
                  key={genre}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => toggleGenre(genre)}
                  className={`rounded-full border px-3 py-1.5 text-sm ${
                    selected
                      ? "border-indigo-400 bg-indigo-600 text-white"
                      : "border-gray-600 text-gray-300 hover:bg-gray-700"
                  }`}
                >
                  {genre}
                </button>
              );
            })}
          </div>
        </fieldset>
      )}

      {step === "artists" && (
        <fieldset>
          <legend className="mb-2 text-sm font-medium text-gray-200">
            Any artists you&rsquo;re into?
          </legend>
          <div className="flex gap-2">
            <input
              type="text"
              aria-label="Add an artist"
              placeholder="e.g. Kendrick Lamar"
              value={artistInput}
              onChange={(e) => setArtistInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addArtist();
                }
              }}
              className="w-full rounded-md border border-gray-600 bg-gray-900 px-3 py-2 text-sm text-white placeholder-gray-500"
            />
            <button
              type="button"
              onClick={addArtist}
              className="rounded-md border border-gray-600 px-3 py-2 text-sm text-gray-200 hover:bg-gray-700"
            >
              Add
            </button>
          </div>
          <ul className="mt-3 flex flex-wrap gap-2">
            {draft.artists.map((artist) => (
              <li key={artist}>
                <button
                  type="button"
                  aria-label={`Remove ${artist}`}
                  onClick={() => removeArtist(artist)}
                  className="rounded-full bg-gray-700 px-3 py-1 text-sm text-gray-200 hover:bg-gray-600"
                >
                  {artist} ×
                </button>
              </li>
            ))}
          </ul>
        </fieldset>
      )}

      {step === "goals" && (
        <fieldset>
          <legend className="mb-2 text-sm font-medium text-gray-200">
            What brings you here?
          </legend>
          <div className="flex gap-2">
            <input
              type="text"
              aria-label="Add a goal"
              placeholder="e.g. audition prep"
              value={goalInput}
              onChange={(e) => setGoalInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addGoal();
                }
              }}
              className="w-full rounded-md border border-gray-600 bg-gray-900 px-3 py-2 text-sm text-white placeholder-gray-500"
            />
            <button
              type="button"
              onClick={addGoal}
              className="rounded-md border border-gray-600 px-3 py-2 text-sm text-gray-200 hover:bg-gray-700"
            >
              Add
            </button>
          </div>
          <ul className="mt-3 flex flex-wrap gap-2">
            {draft.goals.map((goal) => (
              <li key={goal}>
                <button
                  type="button"
                  aria-label={`Remove ${goal}`}
                  onClick={() => removeGoal(goal)}
                  className="rounded-full bg-gray-700 px-3 py-1 text-sm text-gray-200 hover:bg-gray-600"
                >
                  {goal} ×
                </button>
              </li>
            ))}
          </ul>
        </fieldset>
      )}

      {step === "experience" && (
        <fieldset>
          <legend className="mb-2 text-sm font-medium text-gray-200">
            Where are you on the journey?
          </legend>
          <div className="space-y-2">
            {EXPERIENCE_LEVELS.map((level) => {
              const selected = draft.experience === level.value;
              return (
                <button
                  key={level.value}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => setExperience(level.value)}
                  className={`block w-full rounded-md border px-3 py-2 text-left text-sm ${
                    selected
                      ? "border-indigo-400 bg-indigo-600/20 text-white"
                      : "border-gray-600 text-gray-300 hover:bg-gray-700"
                  }`}
                >
                  <span className="font-medium">{level.label}</span>
                  <span className="block text-xs text-gray-400">
                    {level.description}
                  </span>
                </button>
              );
            })}
          </div>
        </fieldset>
      )}

      <div className="mt-6 flex items-center justify-between gap-4">
        <button
          type="button"
          onClick={() => onComplete?.()}
          className="text-sm text-gray-400 hover:text-gray-200"
        >
          Skip for now
        </button>
        <div className="flex items-center gap-2">
          {stepIndex > 0 && (
            <button
              type="button"
              onClick={back}
              disabled={saving}
              className="rounded-md border border-gray-600 px-4 py-2 text-sm text-gray-200 hover:bg-gray-700 disabled:opacity-50"
            >
              Back
            </button>
          )}
          <button
            type="button"
            onClick={next}
            disabled={saving}
            className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
          >
            {isLast ? (saving ? "Saving…" : "Done") : "Next"}
          </button>
        </div>
      </div>
    </section>
  );
}
