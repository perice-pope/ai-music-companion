#!/usr/bin/env bash
set -euo pipefail

# Install system deps for Tauri and xvfb (headless UI tests)
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf \
  libssl-dev \
  libgtk-3-dev \
  xvfb \
  libasound2-dev \
  libjack-jackd2-dev

# Install pnpm
npm install -g pnpm

# Install just (task runner)
cargo install just

# Install cargo tools
cargo install cargo-audit
cargo install tauri-cli --version "^2"

# Install frontend deps
cd apps/desktop && pnpm install || true
