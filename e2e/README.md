# GUI End-to-End Test Harness

UI/IPC-level **"works with no internet"** regression guard for the React
front end. Companion to the library-level Rust E2E harness
(`crates/brain/tests/e2e_offline.rs`): that one proves the *core* works
offline; this one proves the *screens* do.

## Approach (and why)

**Option B — Playwright against the production Vite web build, with the Tauri
`invoke` IPC mocked at the boundary.**

The preferred approach (Option A: `tauri-driver` + WebdriverIO driving the
built native binary) needs an X display and a WebKit WebDriver
(`WebKitWebDriver`), neither of which our headless CI containers provide. So
this suite drives the **built web bundle** instead — the real React screens,
the real Zustand state machine and routing, the real recap rendering — and
replaces only the thin JSON IPC boundary with a recording fake
(`support/tauri-mock.ts`).

This is labelled honestly: **UI E2E against the web build with mocked IPC;
the native Tauri shell is covered separately** (by the Rust harness above and
the `tauri-rust` CI job).

### Why a dedicated mock, not the app's `devShim`

The app ships a `devShim` for browser-preview, but it's `import.meta.env.DEV`
gated and dead-code-eliminated from the production bundle this suite drives.
Our mock additionally **records every IPC call** and **blocks + records every
outbound network attempt**, which `devShim` does not — that recording is what
lets the suite *assert* the offline guarantee.

## What it asserts

1. **Boot** — app loads to the instrument selector; the seeded catalog
   crosses the IPC boundary and renders; **no network**.
2. **Offline-by-default** — no networked surface (sign-in/sync) is reachable
   from the practice path; **no network**. The dedicated *Connections &
   Privacy panel* assertions (every networked toggle defaults OFF + the
   "works offline / what's sent" disclosure copy) are written but
   **auto-skip until that component lands on `main`** — see the note below.
3. **practice → recap** — pick instrument → start → end → recap renders the
   full summary (strengths before areas — the product invariant), with the
   seeded fingerprint dimensions; the whole flow rides over IPC and makes
   **no network**. Live coaching (PR #171) is asserted if the panel is
   present, skipped gracefully otherwise.

### The offline / no-network mechanism

Two independent guards, both must stay clean or the test fails:

- **In-page**: `support/tauri-mock.ts` wraps `fetch` / `XMLHttpRequest` /
  `WebSocket` / `sendBeacon` to record and reject any call. Asserted via
  `window.__netCalls === []`.
- **Route-level**: `fixtures/app.ts` aborts every request whose host isn't
  the local preview origin. Asserted via the aborted-request log being empty.

## Running it

```bash
cd apps/desktop
pnpm e2e:install   # one-time: download Playwright's Chromium
pnpm e2e           # build the web bundle, serve it, run the suite
```

Browserless checks that run **anywhere** (no Chromium needed):

```bash
cd apps/desktop
pnpm e2e:typecheck # strict-TS typecheck of the harness
pnpm e2e:unit      # Vitest checks of the seeded IPC mock + contracts
```

## Known limitation (honest)

The full browser suite requires Playwright's Chromium binary. In sandboxes
where the Playwright browser CDN is firewall-blocked (`cdn.playwright.dev` →
403), `pnpm e2e:install` cannot fetch it and the browser specs cannot
execute. They run normally in GitHub Actions (`.github/workflows/e2e.yml`)
and on any dev machine with network access. The **browserless** `e2e:unit` +
`e2e:typecheck` checks always run and guard the load-bearing mock logic.

## Note: Connections & Privacy panel

`CLAUDE.md` mandates a `ConnectionsPrivacy.tsx` panel surfacing every
networked feature (off by default, with disclosure copy). That component is
**not yet on `main`** — it lives on `claude/offline-first-transparency`. This
suite does not fake it: the panel-specific assertions in
`tests/02-connections-privacy.spec.ts` auto-skip with a clear message and
begin asserting for real (toggles default `aria-checked=false`, disclosure
copy renders) the moment the panel merges with a `connections-privacy-panel`
test id.
