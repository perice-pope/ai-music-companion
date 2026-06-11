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

  it("rejects an unmocked command loudly", async () => {
    const invoke = makeIpcHandler([]);
    await expect(invoke("totally_new_networked_command")).rejects.toThrow(
      /unmocked command/,
    );
  });
});
