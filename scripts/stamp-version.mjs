#!/usr/bin/env node
/**
 * Stamp a release version into the Tauri app sources (#384).
 *
 * semantic-release computes versions and tags, but Tauri stamps installers
 * from `tauri.conf.json` — which nothing updated, so every installer shipped
 * as 0.1.0. This runs as the `@semantic-release/exec` prepare step; the
 * stamped files ride the release commit (`@semantic-release/git`), so the
 * `vX.Y.Z` tag itself points at a stamped tree.
 *
 * Replacements are regex-targeted rather than JSON/TOML re-emits so the
 * diff is one line per file and formatting survives byte-for-byte. Each
 * stamp throws if its pattern is missing — a renamed field must fail the
 * release loudly, never ship a 0.1.0 bundle silently.
 */
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Semver with optional prerelease/build suffix ("2.29.0", "2.29.0-beta.1").
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

/** Top-level `"version"` in tauri.conf.json — the field Tauri bundles from. */
export function stampTauriConf(content, version) {
  const stamped = content.replace(
    /("version":\s*")[^"]+(")/,
    `$1${version}$2`,
  );
  let parsed;
  try {
    parsed = JSON.parse(stamped);
  } catch {
    throw new Error("tauri.conf.json: not valid JSON after stamping");
  }
  if (parsed.version !== version) {
    throw new Error(
      `tauri.conf.json: no "version" field found to stamp (got ${JSON.stringify(parsed.version)})`,
    );
  }
  return stamped;
}

/**
 * The `[package]` version line in Cargo.toml. Line-anchored so inline
 * dependency versions (`tauri-build = { version = "2" }`) never match.
 */
export function stampCargoToml(content, version) {
  const pattern = /^(version = ")[^"]+(")$/m;
  if (!pattern.test(content)) {
    throw new Error("Cargo.toml: no [package] version line found to stamp");
  }
  return content.replace(pattern, `$1${version}$2`);
}

/**
 * The app's own `[[package]]` block in Cargo.lock. Only that block — a
 * dependency sharing the old version string must keep it, and the manifest
 * must stay consistent with the lockfile for `--locked` builds.
 */
export function stampCargoLock(content, version) {
  const pattern =
    /(\[\[package\]\]\nname = "ai-music-companion"\nversion = ")[^"]+(")/;
  if (!pattern.test(content)) {
    throw new Error(
      'Cargo.lock: no [[package]] block for "ai-music-companion" found to stamp',
    );
  }
  return content.replace(pattern, `$1${version}$2`);
}

export function main(version, repoRoot) {
  if (!version || !SEMVER.test(version)) {
    throw new Error(
      `usage: stamp-version.mjs <semver> — got ${JSON.stringify(version ?? null)}`,
    );
  }
  const tauriDir = path.join(repoRoot, "apps", "desktop", "src-tauri");
  const targets = [
    { file: path.join(tauriDir, "tauri.conf.json"), stamp: stampTauriConf },
    { file: path.join(tauriDir, "Cargo.toml"), stamp: stampCargoToml },
    { file: path.join(tauriDir, "Cargo.lock"), stamp: stampCargoLock },
  ];
  for (const { file, stamp } of targets) {
    const content = readFileSync(file, "utf8");
    writeFileSync(file, stamp(content, version));
    console.log(`stamped ${path.relative(repoRoot, file)} -> ${version}`);
  }
}

const invokedDirectly =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  try {
    main(process.argv[2], path.resolve(path.dirname(fileURLToPath(import.meta.url)), ".."));
  } catch (err) {
    console.error(String(err instanceof Error ? err.message : err));
    process.exit(1);
  }
}
