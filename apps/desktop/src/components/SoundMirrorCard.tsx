import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SoundMirrorDto } from "../types/brain";

/** Human copy per resolved axis — absent axes simply don't get a line. */
function axisSentence(dto: SoundMirrorDto): string | null {
  const p = dto.profile;
  if (!p) return null;
  const parts: string[] = [];
  if (p.mode_lean === "minor") parts.push("you lean into darker, minor colors");
  else if (p.mode_lean === "major")
    parts.push("you live in bright, major colors");
  else if (p.mode_lean === "balanced")
    parts.push("you move between light and dark");
  if (p.feel === "swung") parts.push("with a swung, laid-back feel");
  else if (p.feel === "straight") parts.push("with a straight, even pulse");
  else if (p.feel === "mixed")
    parts.push("shifting between straight and swung");
  if (parts.length === 0) return null;
  const s = parts.join(" ");
  return s.charAt(0).toUpperCase() + s.slice(1) + ".";
}

/** How many measured sessions the empty state counts toward. */
export const MIRROR_MIN_SESSIONS = 5;

/**
 * "Your sound" (#258): the player's evolving identity, derived from measured
 * sessions only. Below the threshold it invites rather than guesses; the
 * real-world comparison renders only when the backend grounded one.
 */
export default function SoundMirrorCard() {
  const [dto, setDto] = useState<SoundMirrorDto | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const view = await invoke<SoundMirrorDto>("get_sound_mirror");
        if (!cancelled && view && typeof view.sessions_seen === "number") {
          setDto(view);
        }
      } catch (err) {
        console.error("get_sound_mirror failed:", err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (!dto) {
    return null;
  }

  const sentence = axisSentence(dto);

  return (
    <div
      className="w-64 rounded-lg border border-gray-700 bg-gray-800/70 p-4"
      data-testid="sound-mirror"
    >
      <p className="text-xs font-semibold uppercase tracking-wider text-gray-400">
        Your sound
      </p>
      {!dto.profile || !sentence ? (
        <p className="mt-2 text-sm text-gray-300" data-testid="mirror-empty">
          Keep playing to discover your sound —{" "}
          {Math.min(dto.sessions_seen, MIRROR_MIN_SESSIONS)} of{" "}
          {MIRROR_MIN_SESSIONS} sessions.
        </p>
      ) : (
        <>
          <p className="mt-2 text-sm text-gray-100" data-testid="mirror-axes">
            {sentence}
          </p>
          {dto.profile.comparison ? (
            <p
              className="mt-2 text-sm font-medium text-amber-300"
              data-testid="mirror-comparison"
            >
              {dto.profile.comparison.label} ✨
            </p>
          ) : (
            <p
              className="mt-2 text-xs italic text-gray-400"
              data-testid="mirror-listening"
            >
              still listening…
            </p>
          )}
        </>
      )}
    </div>
  );
}
