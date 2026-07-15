import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { Page } from "@playwright/test";
import { test, expect, assertNoNetwork } from "../fixtures/app";

/**
 * #336 — the .wav loading message must be SEEN, not merely rendered.
 *
 * Three consecutive VA runs (#324, #334, #347) dropped the kit's sample
 * recording on the score screen and never saw "Listening for notes…", while
 * two merged fixes (#323 backend threading, #342 event-independent seed)
 * kept passing their unit tests — those drive the component with mocked
 * plumbing and `waitFor`, which is satisfied by a message that exists for a
 * single microtask. The sample is small enough that the real backend can
 * return before the seeded indicator ever reaches a painted frame:
 * seed-then-clear collapses into one React commit and the tester sees
 * nothing.
 *
 * This suite reproduces the finding through the real flow the playbook's
 * step 12 describes: the built production bundle, the real score screen,
 * a genuine DOM drag-drop of the actual `sample-recording-c-major-scale.wav`
 * from the VA kit, and — crucially — an INSTANT backend (`__importDelayMs`
 * defaults to 0), the worst case the fix must survive. The mock emits no
 * `import-progress` events at all, so these tests also pin #342's contract
 * that the message never depends on event delivery.
 *
 * Caveat, stated honestly: this drives Chromium against the web build, not
 * the packaged WKWebView. If the VA still sees nothing on a build containing
 * this suite, the residue is isolated to the native shell's drop delivery.
 */

const WAV_PATH = resolve(
  __dirname,
  "../../va-testing-kit/samples/sample-recording-c-major-scale.wav",
);
const WAV_NAME = "sample-recording-c-major-scale.wav";

/** Click through to the score screen, exactly like the VA playbook. */
async function gotoScoreScreen(page: Page): Promise<void> {
  await page.goto("/");
  await page.getByTestId("instrument-card-trumpet").click();
  await page.getByTestId("practice-with-score-button").click();
  await expect(page.getByTestId("score-drop-zone")).toBeVisible();
}

/** Drag-drop the real VA-kit sample recording onto the drop zone. */
async function dropSampleWav(page: Page): Promise<void> {
  const wavB64 = readFileSync(WAV_PATH).toString("base64");
  await page.evaluate(
    ({ b64, name }) => {
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
      const file = new File([bytes], name, { type: "audio/wav" });
      const dt = new DataTransfer();
      dt.items.add(file);
      const target = document.querySelector('[data-testid="score-drop-zone"]');
      if (!target) throw new Error("score drop zone not found");
      for (const type of ["dragover", "drop"]) {
        target.dispatchEvent(
          new DragEvent(type, {
            bubbles: true,
            cancelable: true,
            dataTransfer: dt,
          }),
        );
      }
    },
    { b64: wavB64, name: WAV_NAME },
  );
}

test.describe(".wav import loading message (#336)", () => {
  test("an instant transcription still shows a perceivable loading message", async ({
    page,
    abortedRequests,
  }) => {
    await gotoScoreScreen(page);
    // __importDelayMs stays 0: the backend answers within a microtask — the
    // fast-Mac shape the VA could never see a message in.
    await dropSampleWav(page);

    // The loading message appears…
    const loading = page.getByText("Listening for notes…");
    await expect(loading).toBeVisible();
    // …and is still there half a second later — a human can read it, not a
    // zero-frame flash. (This wait IS the assertion: perceivability.)
    await page.waitForTimeout(500);
    await expect(loading).toBeVisible();

    // Then it hands over to the beta-labelled success banner and clears.
    await expect(
      page.getByText(/transcribed from audio \(beta\)/),
    ).toBeVisible();
    await expect(loading).toBeHidden();

    // The real file rode the real drop path: the backend command received
    // the sample's exact name and full byte payload.
    const wavByteLen = readFileSync(WAV_PATH).length;
    const call = await page.evaluate(() => {
      const c = (window.__ipcCalls ?? []).find(
        (i) => i.cmd === "import_audio_file",
      );
      if (!c) return null;
      const args = c.args as
        | { sourceFilename?: string; bytes?: number[] }
        | undefined;
      return {
        name: args?.sourceFilename ?? null,
        byteLen: args?.bytes?.length ?? 0,
      };
    });
    expect(call).toEqual({ name: WAV_NAME, byteLen: wavByteLen });

    // The import (bytes in, score back) never left the device.
    await assertNoNetwork(page, abortedRequests);
  });

  test("the message shows while a slow transcription is pending — no events, no network", async ({
    page,
    abortedRequests,
  }) => {
    await gotoScoreScreen(page);
    await page.evaluate(() => {
      window.__importDelayMs = 1500;
    });
    await dropSampleWav(page);

    // Mid-import: the backend is still working, zero progress events have
    // been emitted, and the message is on screen anyway (#342's contract,
    // now asserted on the real screen).
    await expect(page.getByText("Listening for notes…")).toBeVisible();
    await expect(page.getByText(/transcribed from audio/)).toBeHidden();

    // The whole flow rides IPC — the offline promise holds during import.
    await assertNoNetwork(page, abortedRequests);

    // Completion: beta banner up, loading state gone.
    await expect(page.getByText(/transcribed from audio \(beta\)/)).toBeVisible(
      { timeout: 5000 },
    );
    await expect(page.getByText("Listening for notes…")).toBeHidden();
  });
});
