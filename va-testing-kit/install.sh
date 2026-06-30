#!/usr/bin/env bash
# One-time setup for the AI Music Companion tester.
# Installs the basics (Homebrew, Node, pnpm, Claude Code), installs the /test-app
# skill, and saves the feedback code. After this, the tester just opens Claude
# Code and types /test-app.
#
# Run from the kit folder:   bash install.sh
set -uo pipefail

KIT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_SRC="$KIT_DIR/skills/test-app"
SKILL_DST="$HOME/.claude/skills/test-app"
TOKEN_DIR="$HOME/.config/amc"
TOKEN_FILE="$TOKEN_DIR/feedback_token"

say(){  printf '\n\033[1;36m%s\033[0m\n' "$*"; }
ok(){   printf '\033[32m  ✓ %s\033[0m\n' "$*"; }
info(){ printf '   %s\n' "$*"; }

say "AI Music Companion — Tester Setup"
info "This sets up everything once. It may ask for your Mac password — that's normal and safe."

# --- Homebrew --------------------------------------------------------------
if ! command -v brew >/dev/null 2>&1; then
  [ -x /opt/homebrew/bin/brew ] && eval "$(/opt/homebrew/bin/brew shellenv)"
  [ -x /usr/local/bin/brew ]    && eval "$(/usr/local/bin/brew shellenv)"
fi
if ! command -v brew >/dev/null 2>&1; then
  say "Installing Homebrew (the tool that installs the others)..."
  NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  [ -x /opt/homebrew/bin/brew ] && eval "$(/opt/homebrew/bin/brew shellenv)"
  [ -x /usr/local/bin/brew ]    && eval "$(/usr/local/bin/brew shellenv)"
fi
command -v brew >/dev/null 2>&1 && ok "Homebrew ready"

# --- Node + git ------------------------------------------------------------
if ! command -v node >/dev/null 2>&1; then
  say "Installing Node..."
  brew install node
fi
command -v git  >/dev/null 2>&1 || brew install git
ok "Node $(node -v 2>/dev/null) and git ready"

# --- pnpm via corepack -----------------------------------------------------
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

# --- Install the skill -----------------------------------------------------
say "Installing the /test-app skill..."
mkdir -p "$SKILL_DST/scripts"
cp -f "$SKILL_SRC/SKILL.md" "$SKILL_DST/SKILL.md"
cp -f "$SKILL_SRC/scripts/"*.sh "$SKILL_DST/scripts/"
chmod +x "$SKILL_DST/scripts/"*.sh
ok "Skill installed"

# --- Feedback code ---------------------------------------------------------
say "Feedback code"
if [ -s "$TOKEN_FILE" ]; then
  ok "A feedback code is already saved."
else
  info "Your manager sent you a feedback code (a long line starting with 'github_pat_')."
  printf "   Paste it here and press Enter (or just press Enter to do this later): "
  read -r CODE
  if [ -n "${CODE// /}" ]; then
    mkdir -p "$TOKEN_DIR"
    printf '%s' "$CODE" | tr -d '[:space:]' > "$TOKEN_FILE"
    chmod 600 "$TOKEN_FILE"
    ok "Feedback code saved."
  else
    info "Skipped. You can run this installer again later to add it."
  fi
fi

say "All set! 🎉"
info "To test the app: open Claude Code and type:  /test-app"
