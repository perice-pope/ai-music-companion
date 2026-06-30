#!/usr/bin/env bash
# One-time setup for the AI Music Companion tester.
# Installs the basics (Homebrew, Node, pnpm, Claude Code, git), then installs a
# THIN, self-updating /test-app skill that pulls the latest app + testing kit
# from the repo on every run. After this, the tester just types /test-app — and
# stays up to date automatically as the app and checklist change.
#
# Run it directly:   bash install.sh
# Or one line:       curl -fsSL https://raw.githubusercontent.com/perice-pope/ai-music-companion/main/va-testing-kit/install.sh | bash
set -uo pipefail

REPO_URL="https://github.com/perice-pope/ai-music-companion.git"
AMC_HOME="${AMC_HOME:-$HOME/amc}"
REPO_DIR="$AMC_HOME/ai-music-companion"
KIT="$REPO_DIR/va-testing-kit/skills/test-app"
SKILL_DST="$HOME/.claude/skills/test-app"
TOKEN_DIR="$HOME/.config/amc"; TOKEN_FILE="$TOKEN_DIR/feedback_token"

say(){  printf '\n\033[1;36m%s\033[0m\n' "$*"; }
ok(){   printf '\033[32m  ✓ %s\033[0m\n' "$*"; }
info(){ printf '   %s\n' "$*"; }

say "AI Music Companion — Tester Setup"
info "This sets up everything once. It may ask for your Mac password — that's normal and safe."

# --- Homebrew --------------------------------------------------------------
for b in /opt/homebrew/bin/brew /usr/local/bin/brew; do [ -x "$b" ] && eval "$("$b" shellenv)"; done
if ! command -v brew >/dev/null 2>&1; then
  say "Installing Homebrew..."
  NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  for b in /opt/homebrew/bin/brew /usr/local/bin/brew; do [ -x "$b" ] && eval "$("$b" shellenv)"; done
fi
command -v brew >/dev/null 2>&1 && ok "Homebrew ready"

# --- Node + git ------------------------------------------------------------
command -v node >/dev/null 2>&1 || { say "Installing Node..."; brew install node; }
command -v git  >/dev/null 2>&1 || brew install git
ok "Node $(node -v 2>/dev/null) and git ready"

# --- pnpm ------------------------------------------------------------------
say "Setting up pnpm..."
corepack enable >/dev/null 2>&1 || true
corepack prepare pnpm@10.33.2 --activate >/dev/null 2>&1 || true
ok "pnpm ready"

# --- Claude Code -----------------------------------------------------------
if ! command -v claude >/dev/null 2>&1; then
  say "Installing Claude Code..."
  curl -fsSL https://claude.ai/install.sh | bash || npm install -g @anthropic-ai/claude-code || true
fi
command -v claude >/dev/null 2>&1 && ok "Claude Code ready" || info "Claude Code: open it once and sign in when prompted."

# --- Get the repo (app + testing kit) --------------------------------------
say "Downloading the app + testing kit..."
mkdir -p "$AMC_HOME"
if [ ! -d "$REPO_DIR/.git" ]; then
  git clone --depth 1 "$REPO_URL" "$REPO_DIR" >/dev/null 2>&1 || { info "Download failed — check internet and re-run."; exit 1; }
else
  git -C "$REPO_DIR" fetch --depth 1 origin main >/dev/null 2>&1 || true
  git -C "$REPO_DIR" reset --hard origin/main >/dev/null 2>&1 || true
fi
[ -f "$KIT/SKILL.md" ] && [ -f "$KIT/scripts/amc.sh" ] || { info "Testing kit not found in the repo yet — ask your manager to push va-testing-kit/."; exit 1; }
ok "App + kit downloaded"

# --- Install the THIN, self-updating skill ---------------------------------
say "Installing the /test-app skill..."
mkdir -p "$SKILL_DST/scripts"
cp -f "$KIT/SKILL.md" "$SKILL_DST/SKILL.md"
cp -f "$KIT/scripts/amc.sh" "$SKILL_DST/scripts/amc.sh"
chmod +x "$SKILL_DST/scripts/amc.sh"
ok "Skill installed (it auto-updates the app + checklist on every run)"

# --- Feedback code ---------------------------------------------------------
say "Feedback code"
if [ -s "$TOKEN_FILE" ]; then
  ok "A feedback code is already saved."
else
  info "Your manager sent you a feedback code (a long line starting with 'github_pat_')."
  printf "   Paste it here and press Enter (or just press Enter to do this later): "
  read -r CODE
  if [ -n "${CODE// /}" ]; then
    mkdir -p "$TOKEN_DIR"; printf '%s' "$CODE" | tr -d '[:space:]' > "$TOKEN_FILE"; chmod 600 "$TOKEN_FILE"
    ok "Feedback code saved."
  else
    info "Skipped. Run this installer again later to add it."
  fi
fi

say "All set! 🎉"
info "To test the app: open Claude Code and type:  /test-app"
info "For a real-sound test (mic + upload + AI feedback):  /test-app desktop"
