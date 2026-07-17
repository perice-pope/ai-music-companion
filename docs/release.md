# Release & Packaging Runbook

How AI Music Companion gets from a commit on `main` to signed, auto-updating
installers on every desktop platform.

> **TL;DR** — `semantic-release.yml` cuts the version tag on push to `main`;
> the `vX.Y.Z` tag triggers `release.yml`, which builds the installers with
> [`tauri-action`](https://github.com/tauri-apps/tauri-action) and uploads them
> to a **draft** GitHub Release. A human reviews, smoke-tests, and publishes.

---

## In plain English (read this first)

This is the developer/maintainer guide for turning the code into the actual
**installers** people download (the `.dmg` for Mac, the `.exe`/`.msi` for
Windows, the Linux packages), and for the **auto-update** that quietly offers
existing users the next version.

**Who runs this:** someone with developer access **and** the paid signing
accounts (an Apple Developer membership and a Windows code-signing certificate).
Without those, the app can still be built, but the computer will show a "this is
from an unidentified developer" warning when people install it.

**The whole flow in one breath:** you merge your changes → the system
automatically picks the new version number and tags it → that tag makes three
robots (one per operating system) build the installers and put them in a
**draft** release → a person downloads them, checks they work, and clicks
**Publish**.

**What a non-developer can do here:** run the build (step 4) to *see* installers
get produced, and do the smoke-test + Publish at the end. **What needs a
developer with accounts:** setting up the one-time signing secrets (section 3)
and the updater key (section 6). Those are clearly marked.

> Pair this with [`qa-runbook.md`](./qa-runbook.md) — that one (written for
> anyone, no coding needed) is how you confirm a build actually *works* on real
> hardware before you publish it.

---

## 1. Architecture of the release pipeline

| Stage | Workflow | Trigger | Output |
| --- | --- | --- | --- |
| Decide version + cut tag | `.github/workflows/semantic-release.yml` | push to `main` | `vX.Y.Z` tag, changelog, GitHub Release shell |
| Build installers | `.github/workflows/release.yml` | push of `v*` tag (or manual `workflow_dispatch`) | `.dmg`/`.app`, `.msi`/`.exe`, `.deb`/`.AppImage`, `latest.json` |
| Publish | manual | human clicks **Publish** on the draft release | public download + updater feed |

Bundle config lives in `apps/desktop/src-tauri/tauri.conf.json`:

- **productName**: `AI Music Companion`
- **identifier**: `com.aimusiccompanion.desktop`
- **targets**: `app`, `dmg` (macOS) · `nsis`, `msi` (Windows) · `deb`, `appimage` (Linux)
- **category**: `Music` · **copyright** set
- **icons**: `icons/icon.icns` (macOS), `icons/icon.ico` (Windows), PNGs (Linux),
  generated with `pnpm tauri icon`.

The version is sourced from `tauri.conf.json` `version`. The release pipeline
keeps it aligned automatically: semantic-release's prepare step runs
`scripts/stamp-version.mjs`, which writes the new version into
`tauri.conf.json`, `src-tauri/Cargo.toml`, and `src-tauri/Cargo.lock`, and the
release commit carries all three (#384 — before this, every installer shipped
labeled 0.1.0). `release.yml` refuses to build a tag whose tree isn't stamped
with the tag's own version.

---

## 2. Prerequisites (local, for maintainers cutting/testing a release)

- Rust stable toolchain + `cargo`.
- Node 20 + `pnpm` (`corepack enable` or `npm i -g pnpm`).
- Platform bundler tooling (only needed if building locally rather than in CI):
  - **macOS**: Xcode command-line tools.
  - **Windows**: WiX/NSIS are fetched by the Tauri CLI on first build.
  - **Linux**: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf`,
    plus `libasound2-dev libssl-dev`.
- A local build smoke-test: `cd apps/desktop && pnpm tauri build`.

---

## 3. Required CI secrets

All signing is **gated on secrets** — if a secret is absent, the corresponding
step is skipped and the build proceeds **unsigned** (Linux never needs signing).
Configure these in **Settings → Secrets and variables → Actions**.

### Tauri updater signing (required for working auto-update)

| Secret | What it is | How to obtain |
| --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Private half of the updater keypair (base64) | `pnpm tauri signer generate -w ~/.tauri/aimc-updater.key` — paste the file contents |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password set when generating the key | chosen at generation time |

The **public** key from that command goes into `tauri.conf.json` →
`plugins.updater.pubkey` (replace `REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY`).
**Never commit the private key.** See §6.

### Apple code signing + notarization (macOS — required to ship outside dev)

| Secret | What it is | How to obtain |
| --- | --- | --- |
| `APPLE_CERTIFICATE` | Base64 of a "Developer ID Application" `.p12` | Apple Developer account → Certificates → export `.p12` from Keychain → `base64 -i cert.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting the `.p12` | set during export |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` | `security find-identity -v -p codesigning` |
| `APPLE_ID` | Apple ID email used for notarization | your Apple Developer account |
| `APPLE_PASSWORD` | App-specific password for that Apple ID | appleid.apple.com → Sign-In & Security → App-Specific Passwords |
| `APPLE_TEAM_ID` | 10-char Apple Developer Team ID | Apple Developer → Membership |

Requires a **paid Apple Developer Program membership** ($99/yr).

### Windows code signing (Windows — required to avoid SmartScreen warnings)

| Secret | What it is | How to obtain |
| --- | --- | --- |
| `WINDOWS_CERTIFICATE` | Base64 of a code-signing `.pfx` | buy an OV/EV code-signing cert from a CA (DigiCert, Sectigo, …) → export `.pfx` → `base64` |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password protecting the `.pfx` | set during export |

`GITHUB_TOKEN` is provided automatically by Actions — no setup needed.

---

## 4. How to cut a release

1. **Land changes on `main`** using Conventional Commits. `feat:` → minor,
   `fix:` → patch, `feat!:`/`BREAKING CHANGE` → major. `semantic-release.yml`
   computes the next version automatically.
2. **Versions align themselves.** The `@semantic-release/exec` prepare step runs
   `scripts/stamp-version.mjs`, stamping the new version into
   `apps/desktop/src-tauri/tauri.conf.json`, `Cargo.toml`, and `Cargo.lock`;
   `@semantic-release/git` commits them with the changelog. If you cut a tag by
   hand, run the stamp yourself first — `release.yml` fails any `v*` tag whose
   tree doesn't carry the tag's version. (This also means tags cut before the
   stamp existed — v2.28.x and older — can no longer be rebuilt by re-running
   `release.yml`; that is deliberate.)
3. **The tag is pushed automatically.** On push to `main`, semantic-release tags
   `vX.Y.Z`. To cut one by hand instead:
   ```bash
   git tag v1.2.3 && git push origin v1.2.3
   ```
4. **Installers build.** The `v*` tag triggers `release.yml`, which fans out
   across `macos-latest`, `windows-latest`, `ubuntu-latest`, builds every
   bundle target, and uploads them to a **draft** GitHub Release together with
   `latest.json`.
5. **Review & publish.** Download the draft's installers, smoke-test each
   platform, then click **Publish release**. Publishing makes the downloads
   public and (because `latest.json` lives at the release's
   `latest/download/latest.json`) switches on auto-update for existing users.

You can also run `release.yml` manually via **Actions → Release (Installers) →
Run workflow** (`workflow_dispatch`) to build installers without cutting a
version — useful for testing the bundle pipeline.

---

## 5. How auto-update works

- The app embeds the **public** updater key and an **endpoint** URL
  (`plugins.updater` in `tauri.conf.json`). The endpoint points at
  `https://github.com/perice-pope/ai-music-companion/releases/latest/download/latest.json`.
- At release time, `tauri-action` (with `includeUpdaterJson: true` and
  `TAURI_SIGNING_PRIVATE_KEY` set) generates `latest.json` — a manifest listing
  the version, per-platform installer URLs, and a **detached signature** for each
  artifact, produced with the private key.
- On the client, the updater downloads `latest.json`, compares versions, and
  verifies the signature against the embedded public key **before** applying any
  update. A signature it can't verify is rejected — this is what prevents a
  compromised release host from pushing a malicious update.
- **Offline-first guarantee (per CLAUDE.md):** the updater performs **no network
  I/O on startup**. The Rust plugin is registered (`tauri_plugin_updater`) but a
  check only happens when the UI explicitly invokes it, and the config sets
  `dialog: true` so the user is prompted before any download. The core
  capture → analysis → recap loop never depends on the network.
  > **Follow-up for the Face/privacy owner:** surface the (opt-in) update check
  > in `ConnectionsPrivacy.tsx` and list it in
  > `docs/architecture/offline-first-and-network-transparency.md`. This PR does
  > not touch those files (not owned here).

---

## 6. What CANNOT run in CI without real certs + a human

These steps require physical/credentialed assets that **cannot exist in a
generic CI runner or this development container** — they need a human with the
right accounts and hardware:

- **Apple notarization & "Developer ID" signing.** Needs a paid Apple Developer
  membership, a `.p12` exported from a real keychain, and Apple's notary service.
  Without the `APPLE_*` secrets the macOS `.dmg`/`.app` build **unsigned**;
  Gatekeeper will warn users until a human supplies real Apple credentials.
- **Windows Authenticode signing.** Needs a code-signing certificate purchased
  from a CA (and, for EV certs, often a hardware token). Without the
  `WINDOWS_CERTIFICATE` secrets the `.msi`/`.exe` build **unsigned**; SmartScreen
  will warn users.
- **Updater keypair generation.** `tauri signer generate` must be run **once by a
  human** on a trusted machine; the private key is stored only as the
  `TAURI_SIGNING_*` CI secrets and the public key pasted into config. It is never
  committed and never generated in CI.

Everything short of those — bundle config, the multi-platform build matrix,
unsigned Linux `.deb`/`.AppImage`, draft-release upload, and the updater wiring —
runs without any of these secrets.

---

## 7. Verified vs. not (development container)

Confirmed green in the Linux dev container on this branch:

- `cargo check --all-targets` (with `tauri-plugin-updater` added) — **pass**
- `cargo clippy --all-targets -- -D warnings` — **clean**
- `pnpm build` (frontend) — **pass**
- `pnpm lint` + `npx tsc --noEmit` — **clean**
- `pnpm tauri info` parses `tauri.conf.json` and detects both updater plugins — **pass**
- Both workflow YAMLs parse — **pass**

**Not runnable here (documented, not faked):**

- A full `pnpm tauri build` to produce real installers — needs the per-OS
  bundlers and would only produce **unsigned** artifacts. Run it in CI (or
  locally per-platform) once secrets are configured.
- Any actual signing/notarization — see §6.
