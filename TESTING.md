# Testing (macOS)

This guide gets a new contributor from a fresh macOS machine to a running app and
a green test suite. All commands are copy/paste-ready for **zsh** and use **pnpm**
(not npm or yarn).

The repo supports two run modes:

- **Desktop app (Tauri)** — Rust owns the microphone; macOS Privacy & Security
  prompts apply to the `.app`. This is the primary mode.
- **Web UI only (Vite in Chrome)** — useful for frontend-only iteration. The
  browser only gets the mic if the UI calls `getUserMedia`.

> **Repo path used throughout this doc:**
> `/Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant`
>
> Every "from the repo root" command assumes you have `cd`'d there:
>
> ```zsh
> cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant
> ```

---

## 1. Prerequisites (one-time setup)

Install in this order. If a step says "verify," run the verification command and
make sure it prints a version before moving on.

### 1a. Xcode Command Line Tools

Required for Rust, Tauri, and most native build steps.

```zsh
xcode-select --install
```

Verify:

```zsh
xcode-select -p
```

### 1b. Homebrew

```zsh
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

After install, follow the post-install instructions Homebrew prints (it adds
`brew` to your `PATH` in `~/.zprofile`). Then open a new terminal and verify:

```zsh
brew --version
```

### 1c. Node 20+ and pnpm

Use Node 20 LTS or newer. The repo pins pnpm via the `packageManager` field
(`pnpm@10.33.2`), so installing Node + enabling Corepack is enough — Corepack
will fetch the exact pnpm version.

```zsh
brew install node
corepack enable
corepack prepare pnpm@10.33.2 --activate
```

Verify:

```zsh
node -v && pnpm -v
```

You should see Node `v20.x` (or newer) and pnpm `10.33.2`.

> If you have multiple Node installs (`nvm` + Homebrew, etc.), confirm `node -v`
> and `pnpm -v` come from the same shell environment. Mixing them causes
> confusing "command not found" errors.

### 1d. Rust (stable toolchain)

```zsh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"
```

The `-y` flag skips the interactive 1/2/3 menu and installs the default
(recommended) profile.

Verify:

```zsh
rustc --version && cargo --version
```

### 1e. `just`, `cargo-audit`, and `tauri-cli`

- `just` runs the recipes in `justfile` (`just ci`, `just test`, `just dev`, etc.).
- `cargo-audit` is required by `just audit` and `just ci`.
- `tauri-cli` (cargo binary) is required by `just dev` and `just build-app`.
  The `justfile` invokes it as `cargo tauri dev` / `cargo tauri build`, so a
  cargo-installed CLI is needed even though `@tauri-apps/cli` is also a pnpm
  dev dependency.

```zsh
brew install just
cargo install cargo-audit
cargo install tauri-cli --version "^2.0.0" --locked
```

The two `cargo install` lines compile from source and can take a few minutes
each. Verify:

```zsh
just --version && cargo audit --version && cargo tauri --version
```

---

## 2. Install project dependencies (one-time)

The frontend `package.json` lives in `apps/desktop` (there is no root
`package.json` or pnpm workspace), so `pnpm install` must run from there:

```zsh
cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant/apps/desktop && pnpm install
```

This installs the frontend deps and pulls the pinned `@tauri-apps/cli`. Rust
dependencies are fetched lazily on the first `cargo` / `just dev` / `just test`
run.

---

## 3. Verify the toolchain (recommended sanity check)

From the repo root, run the full CI pipeline locally. This is the same
pipeline CI runs and is the single best way to confirm everything works:

```zsh
cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant && just ci
```

`just ci` runs `fmt`, `lint`, `test`, `audit`, and the frontend build. It
should end with `✓ CI pipeline passed`.

If you only want a quick test pass:

```zsh
cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant && just test
```

Other useful recipes (all run from the repo root):

| Recipe            | What it does                                           |
| ----------------- | ------------------------------------------------------ |
| `just test`       | `cargo test --workspace` + `pnpm test`                 |
| `just test-rust`  | Rust workspace tests only                              |
| `just test-frontend` | Vitest only                                         |
| `just fmt`        | `cargo fmt` + `pnpm format`                            |
| `just lint`       | `cargo clippy --deny warnings` + `pnpm lint`           |
| `just audit`      | `cargo audit` + `pnpm audit`                           |
| `just bench`      | Latency benchmarks (must stay <25 ms — see CLAUDE.md)  |
| `just dev`        | Run the desktop app in dev mode                        |
| `just build-app`  | Build a distributable `.app` / `.dmg`                  |

---

## 4. Run the desktop app (Tauri) — primary dev workflow

This is the mode where the Rust backend owns the mic and macOS permission
prompts apply.

From the repo root:

```zsh
cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant && just dev
```

`just dev` already starts Vite (via Tauri's `beforeDevCommand`) and the native
window — you do **not** need to run `pnpm dev` separately.

What you should see:

1. Terminal: Vite starts on `http://localhost:1420`.
2. Terminal: Rust compiles `crates/ears`, `crates/brain`, and the Tauri shell
   (first build is slow — several minutes is normal).
3. A **native desktop window** titled "AI Music Companion" opens.

### Triggering the macOS microphone prompt

macOS prompts the first time the app actually opens the mic — typically when
you start a practice session, **not** when the window first appears.

Checklist:

1. Quit any app that may be holding the mic (Microsoft Teams, Zoom, etc.).
2. Run `just dev`.
3. In the app UI, **start a practice session**. macOS should now prompt.
4. After approving, confirm the app appears in
   **System Settings → Privacy & Security → Microphone**.

If you never see a prompt and the app does not appear in the Microphone list,
reset the decision and try again:

```zsh
tccutil reset Microphone com.ai-music-companion.app
```

Then re-run `just dev` and start a session again. (`com.ai-music-companion.app`
is the bundle identifier defined in `apps/desktop/src-tauri/tauri.conf.json`.)

### Build a distributable desktop app

From the repo root:

```zsh
cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant && just build-app
```

Artifacts land under `apps/desktop/src-tauri/target/release/bundle/`. The
`.app` is in `macos/` and the `.dmg` is in `dmg/`.

---

## 5. Run the web UI only (Chrome) — frontend-only workflow

Use this only when you do not need the Rust backend. From the repo root:

```zsh
cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant/apps/desktop && pnpm dev
```

Then open the URL Vite prints — it will be **`http://localhost:1420`**
(the port is pinned in `vite.config.ts`).

### Mic permissions in Chrome

Chrome only prompts for microphone access when JavaScript calls
`navigator.mediaDevices.getUserMedia({ audio: true })`. If the UI does not
call that API, Chrome will not prompt and macOS will not list anything for
this repo under **System Settings → Privacy & Security → Microphone**.

If the UI does call `getUserMedia` and you need to manage the permission:

- Click the **site info** icon in the address bar → **Site settings** →
  set **Microphone** to **Allow**.
- Or open `chrome://settings/content/microphone` and pick the right device.

---

## 6. Sanity checklist

Run these from the repo root to confirm a healthy setup. Each line should
succeed (non-zero exit codes mean something is off).

```zsh
cd /Users/pericepope/Documents/Claude/Projects/AI_Practice_Assistant
node -v
pnpm -v
rustc --version
cargo --version
just --version
cargo audit --version
cargo tauri --version
(cd apps/desktop && pnpm install)
just ci
```

Then verify the apps actually run:

- [ ] `just dev` opens a native window and Vite serves on `http://localhost:1420`.
- [ ] Starting a practice session triggers the macOS mic prompt the first time.
- [ ] After approving, the app appears under **Privacy & Security → Microphone**.
- [ ] `just build-app` produces a `.app` under
      `apps/desktop/src-tauri/target/release/bundle/macos/`.

If any step fails, re-read the matching section above before opening an issue.
