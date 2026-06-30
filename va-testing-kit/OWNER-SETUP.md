# Owner setup (you, once)

Goal: your VA types **one thing** (`/test-app`) and it sets up the latest app, walks her
through testing, and files her feedback as a GitHub issue on `perice-pope/ai-music-companion`.
She does **not** need a GitHub account — feedback is filed using a narrow token you create.

## 1. Create the feedback token (2 minutes)

This token lets the skill open issues on **only** this one repo, nothing else. Revocable anytime.

1. Go to **https://github.com/settings/personal-access-tokens/new** (Fine-grained token).
2. **Token name:** `amc-va-feedback`
3. **Expiration:** 90 days (or your preference).
4. **Resource owner:** `perice-pope`
5. **Repository access:** *Only select repositories* → choose **ai-music-companion**.
6. **Permissions → Repository permissions → Issues:** set to **Read and write**.
   (Leave everything else as *No access*.)
7. Click **Generate token** and copy it. It looks like `github_pat_XXXX…`.

> All issues filed by the VA will appear authored by **you** (the token owner), titled
> `[VA Test] <date> — <commit>`. You'll get the normal GitHub notification.

## 2. Get the kit + token onto her Mac

Pick whichever is easier:

**Option A — Send the folder.** Zip `amc-va-testing-kit/` and AirDrop/email it to her.
Tell her to unzip it, then (per `VA-README.md`) open Terminal and run `bash install.sh`
from inside the folder. During install it asks her to paste the **feedback token** — send
her the token separately (text/email).

**Option B — One-liner (if you commit the kit to the repo).** Put this `amc-va-testing-kit/`
folder in the repo at `va-testing-kit/`, push it, then she pastes this in Terminal:

```
curl -fsSL https://raw.githubusercontent.com/perice-pope/ai-music-companion/main/va-testing-kit/install.sh | bash
```

…but with Option B she still needs the token, so the installer will prompt her for it.
(The token is **never** stored in the repo — only on her machine, in `~/.config/amc/feedback_token`.)

## 3. That's it

From then on, every time she opens Claude Code and types `/test-app`, the skill:
1. pulls the latest `main`,
2. runs the web UI in Chrome (`pnpm dev` → http://localhost:1420),
3. walks her through a friendly 6-point checklist,
4. files her answers as a GitHub issue you can read in the repo's **Issues** tab.

### Two modes

- **`/test-app`** (default) — fast **web preview** in Chrome. Sample data, **no real microphone**
  (the audio engine is Rust-only and isn't running). Perfect for look/flow/usability feedback.
- **`/test-app desktop`** — the **real native app** with live mic and pitch coaching. The **first**
  desktop run installs Rust + the Tauri builder on her Mac and **compiles for 10–30+ minutes**, and
  macOS will prompt for the microphone. Later runs are faster. Use this when you want a real-audio pass.
  (The web installer does **not** pre-install the desktop toolchain — it's fetched on demand the first
  time she runs desktop, so the simple path stays fast.)

## Maintenance
- **Rotate/revoke the token:** GitHub → Settings → Developer settings → Fine-grained tokens.
  Re-run her installer (or have her re-run it) to paste a new one.
- **Add AI coaching during tests (optional):** set `MUSIC_COMPANION_LLM_API_KEY` in her
  environment. Without it the app runs fine in offline mode (no live coaching tips).
- **Try it yourself first:** on your machine the skill is already installed. Open Claude Code
  here and type `/test-app`. Your machine has no token file, so feedback falls back to filing
  via your `gh` login automatically.
