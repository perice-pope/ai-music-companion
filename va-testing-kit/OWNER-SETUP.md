# Owner setup (you, once)

Goal: your VA types **one thing** (`/test-app`) and it sets up the latest app, walks her through
testing, and files her feedback as a GitHub issue. She needs **no GitHub account** — feedback is
filed with a narrow token you create. And it's **self-updating**: as you change the app *or* the
testing checklist, her next run is automatically current — no re-install.

## How it stays up to date

The kit lives in the repo at `va-testing-kit/`. The skill installed on her Mac is a thin
bootstrapper: every `/test-app` run it pulls the latest `main` (app **and** kit), then follows the
live checklist at `va-testing-kit/PLAYBOOK.md`. So:

- Change the **app** → she tests the newest build next run.
- Change **`va-testing-kit/PLAYBOOK.md`** (the questions she's asked) or the **samples** → she gets
  the new flow next run. **Edit the playbook to change what she tests.**
- She only re-runs the installer if the tiny bootstrap itself changes (rare).

## 1. Create the feedback token (2 min)

Lets the skill open issues on **only** this repo. Revocable anytime.

1. https://github.com/settings/personal-access-tokens/new (fine-grained)
2. Name `amc-va-feedback`; Expiration 90 days; Resource owner `perice-pope`
3. Repository access → **Only select repositories** → **ai-music-companion**
4. Permissions → Repository → **Issues: Read and write** (everything else: No access)
5. Generate, copy the `github_pat_…` string.

Issues appear authored by you, titled `[VA Test] <date> — <mode> — <commit>`.

## 2. Get her set up

She has no GitHub account, so cloning (public) is fine and only the token is sensitive — it's typed
once during install and stored locally on her Mac (`~/.config/amc/feedback_token`), never in the repo.

- **One-liner (simplest):** send her this for Terminal, plus the token separately:
  ```
  curl -fsSL https://raw.githubusercontent.com/perice-pope/ai-music-companion/main/va-testing-kit/install.sh | bash
  ```
- **Or the seed zip** (`VA-README.md` inside it): if the curl line is awkward, send the small seed
  zip; her `install.sh` does the same thing.

Either way, after install she just types `/test-app`.

## 3. The two modes

- **`/test-app`** — fast web preview in Chrome. Sample data, **no mic, no file import**. Only good
  for judging look / wording / screen flow.
- **`/test-app desktop`** — the **real** app: live mic, music-file upload, and AI critique. **This is
  the only mode that can test the two priority features** (upload-and-practice, and the AI hearing &
  critiquing). First run installs Rust + the Tauri builder + the audio engine and compiles for
  10–30+ min; later runs are faster. She'll be asked to play/hum a few notes.

## 4. Optional — turn on real AI coaching during tests

The on-device analysis (pitch, tone, intonation, groove) works with no key. The **conversational
coaching tips and recap narration** need an Anthropic API key. To enable on her Mac, put your key in
`~/.config/amc/llm_key` (the desktop runner reads it and sets `MUSIC_COMPANION_LLM_API_KEY`). Costs
apply per your Anthropic account, so only do this when you want to test the worded feedback.

## Maintenance
- **Change what she tests:** edit `va-testing-kit/PLAYBOOK.md` and push. No re-install.
- **Rotate/revoke token:** GitHub → Developer settings → Fine-grained tokens; have her re-run the
  installer to paste a new one.
- **Try it yourself:** the skill is installed on your machine — type `/test-app`. With no token file,
  feedback falls back to filing via your `gh` login.
