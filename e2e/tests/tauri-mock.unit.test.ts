/**
 * Browserless unit checks of the seeded IPC handler.
 *
 * The full GUI suite needs a real browser (Playwright Chromium) to drive
 * the React app. That binary cannot be fetched in some locked-down
 * containers (the Playwright browser CDN is firewall-blocked), so these
 * Vitest checks exist to give the *load-bearing* mock logic genuine,
 * runnable coverage everywhere — including environments where the browser
 * download is blocked. They run under `pnpm e2e:unit`.
 *
 * They assert the seeded responses match the IPC contracts the React
 * screens consume, and that an unknown command rejects loudly (so a new
 * IPC dependency in the offline flow can't slip past unmocked).
 */
import { describe, it, expect } from "vitest";
import {
  makeIpcHandler,
  SEEDED_INSTRUMENTS,
  SEEDED_RECAP,
  SEEDED_EMPTY_RECAP,
  SEEDED_SCORE,
  type RecordedInvoke,
} from "../support/tauri-mock";

describe("seeded Tauri IPC handler", () => {
  it("records every call in order", async () => {
    const log: RecordedInvoke[] = [];
    const invoke = makeIpcHandler(log);
    await invoke("list_instruments");
    await invoke("start_practice_session", { instrument: "Trumpet" });
    expect(log.map((c) => c.cmd)).toEqual([
      "list_instruments",
      "start_practice_session",
    ]);
    expect(log[1].args).toEqual({ instrument: "Trumpet" });
  });

  it("returns the seeded instrument catalog with the camelCase wire shape", async () => {
    const invoke = makeIpcHandler([]);
    const list = (await invoke(
      "list_instruments",
    )) as typeof SEEDED_INSTRUMENTS;
    expect(list).toEqual(SEEDED_INSTRUMENTS);
    for (const inst of list) {
      // Mirrors `commands::InstrumentInfo` (rename_all = camelCase).
      expect(inst).toHaveProperty("freqMinHz");
      expect(inst).toHaveProperty("freqMaxHz");
      expect(inst).toHaveProperty("vibratoToleranceCents");
      expect(typeof inst.emoji).toBe("string");
    }
  });

  it("returns a recap whose shape matches the SessionRecap contract", async () => {
    const invoke = makeIpcHandler([]);
    const recap = (await invoke("end_practice_session")) as typeof SEEDED_RECAP;
    // Strengths-before-areas is a UI invariant, but the data must carry both.
    expect(recap.strengths.length).toBeGreaterThan(0);
    expect(recap.areas_to_improve.length).toBeGreaterThan(0);
    expect(recap.phrase_count).toBeGreaterThan(0); // ⇒ full summary variant
    expect(recap.fingerprint?.intonation?.note_count).toBeGreaterThan(0);
    expect(recap.fingerprint?.groove?.tempo_bpm).toBeGreaterThan(0);
  });

  it("hands out distinct session ids", async () => {
    const invoke = makeIpcHandler([]);
    const a = await invoke("start_practice_session");
    const b = await invoke("start_practice_session");
    expect(a).not.toEqual(b);
  });

  it("returns the duration-aware empty recap for a Voice session (#185/#186)", async () => {
    const invoke = makeIpcHandler([]);
    await invoke("start_practice_session", { instrument: "Voice" });
    const recap = (await invoke(
      "end_practice_session",
    )) as typeof SEEDED_EMPTY_RECAP;
    expect(recap.phrase_count).toBe(0); // ⇒ empty variant
    expect(recap.duration_secs).toBeGreaterThan(0); // real time, not floored
    // Never the accusatory copy when the session actually ran.
    expect(recap.overall_assessment).not.toMatch(/didn't get to play/i);
    expect(recap.overall_assessment).toMatch(/pick out distinct phrases/i);
  });

  it("returns the full summary recap for a non-Voice session", async () => {
    const invoke = makeIpcHandler([]);
    await invoke("start_practice_session", { instrument: "Trumpet" });
    const recap = (await invoke("end_practice_session")) as typeof SEEDED_RECAP;
    expect(recap.phrase_count).toBeGreaterThan(0);
  });

  it("serves a seeded score library and a loadable score (#184)", async () => {
    const invoke = makeIpcHandler([]);
    const scores = (await invoke("list_scores")) as unknown[];
    expect(scores.length).toBeGreaterThan(0);
    const loaded = (await invoke("get_score", { id: SEEDED_SCORE.id })) as {
      entry: typeof SEEDED_SCORE;
      music_xml: string;
    };
    expect(loaded.entry.id).toBe(SEEDED_SCORE.id);
    expect(loaded.music_xml).toContain("score-partwise");
  });

  it("serves an audio import the screens can immediately get_score (#336)", async () => {
    const invoke = makeIpcHandler([]);
    const imported = (await invoke("import_audio_file", {
      sourceFilename: "sample-recording-c-major-scale.wav",
      bytes: [1, 2, 3],
    })) as {
      entry: { id: string; source_filename: string };
      polyphonic: boolean;
      low_confidence: boolean;
    };
    // Clean monophonic result — the drop zone must show the loading message
    // and the plain beta banner, never the quality warning.
    expect(imported.polyphonic).toBe(false);
    expect(imported.low_confidence).toBe(false);
    // The entry echoes the dropped file, and the follow-up get_score the
    // store issues loads a renderable score (the seeded mock serves the
    // same score for every id — this pins "loadable", not id routing).
    expect(imported.entry.source_filename).toBe(
      "sample-recording-c-major-scale.wav",
    );
    const loaded = (await invoke("get_score", { id: imported.entry.id })) as {
      music_xml: string;
    };
    expect(loaded.music_xml).toContain("score-partwise");
  });

  it("rejects an unmocked command loudly", async () => {
    const invoke = makeIpcHandler([]);
    await expect(invoke("totally_new_networked_command")).rejects.toThrow(
      /unmocked command/,
    );
  });
});
