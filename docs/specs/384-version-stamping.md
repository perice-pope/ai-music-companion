# Spec: Releases stamp the app version; the app shows it (#384)

> From VA run 2026-07-14 (#381) and 2026-07-16 (#401): every build — dev and packaged —
> reports **0.1.0**, findable only in the Mac menu bar. We shipped v2.28.x.

## 1. Summary
Make each semantic-release write its version into the Tauri app sources so installers are
correctly versioned, guard the installer build against unstamped trees, and surface the
running version unobtrusively in-app so beta testers can file useful reports.

## 2. Problem / why
`semantic-release.yml` computes the version, tags, and publishes — but never touches
`apps/desktop/src-tauri/tauri.conf.json` or `Cargo.toml`, which both still say `0.1.0`.
Tauri stamps the bundle from `tauri.conf.json`, so every installer and the About box say
0.1.0. There is also no in-app version display at all; the VA had to dig through the Mac
menu bar and still got a useless number (#384).

## 3. Non-goals
- No auto-updater changes (`latest.json` flows are untouched; correct versions make them
  *more* correct for free).
- No stamping of workspace crates (`crates/ears`, `crates/brain`) — internal, not user-facing.
- No new Settings screen; the version rides existing surfaces (picker footer,
  Connections & Privacy).

## 4. Contract / interface
- `scripts/stamp-version.mjs <semver>` — rewrites the version in
  `apps/desktop/src-tauri/tauri.conf.json`, `apps/desktop/src-tauri/Cargo.toml`, and
  `apps/desktop/src-tauri/Cargo.lock` (the `ai-music-companion` package block only),
  preserving every other byte. Exports pure `stampTauriConf` / `stampCargoToml` /
  `stampCargoLock` (content, version) → content for tests. Non-zero exit + clear message
  on bad/missing version or unmatched pattern.
- `.releaserc` — `@semantic-release/exec` `prepareCmd` runs the stamp; `@semantic-release/git`
  commits the three stamped files with the release commit, so the `vX.Y.Z` tag points at a
  stamped tree and `release.yml`'s tag checkout builds correctly-versioned installers.
- `release.yml` — tag builds assert `tauri.conf.json`/`Cargo.toml` version == tag version
  before building; mismatch fails red.
- `<AppVersionBadge />` — renders `v{getVersion()}` (Tauri app API, cannot drift from the
  bundle); renders nothing while unknown or when the shell API is absent/failing.

## 5. Acceptance criteria (numbered, testable)
1. Stamping `2.29.0` updates the version field in all three files and changes nothing else
   (formatting, comments, other packages' versions preserved).
2. Stamping is idempotent: running twice with the same version succeeds and is a no-op the
   second time.
3. The script fails (non-zero, named file in the message) when: version arg missing or not
   semver; a target file's version pattern is missing (e.g. the lockfile block renamed).
4. Only the `ai-music-companion` block in `Cargo.lock` changes — a dependency that shares
   the old version string keeps it.
5. Tag-triggered installer builds fail red when the checked-out tree's version ≠ the tag's
   (the exact 0.1.0 regression), and pass when they match.
6. The instrument picker and Connections & Privacy each show `v<version>` read from
   `getVersion()` once it resolves.
7. When `getVersion()` rejects (browser preview, shell absent), the badge renders nothing —
   no error text, no crash.

## 6. Edge cases & failure modes
- Prerelease/build-metadata versions (`2.29.0-beta.1`) — accepted by the semver check.
- `tauri.conf.json` gains a second `"version"` key someday (e.g. plugin config) → only the
  top-level key is replaced; the JSON-parse verification asserts the parsed `version`.
- `Cargo.toml` dependency tables contain `version = "2"` inline — the stamp matches only the
  line-anchored `[package]` field.
- `getVersion()` pending forever (never resolves) → badge stays absent; nothing blocks.
- Component unmounts before `getVersion()` resolves → no setState-after-unmount.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `stampVersion.test.ts` — "stamps all three files" | new version present, rest byte-identical |
| AC2 | "is idempotent" | second run returns identical content |
| AC3 | "rejects bad versions / unmatched patterns" | throws with file/arg named |
| AC4 | "leaves other packages' versions alone" | dependency block with same old version untouched |
| AC5 | manual: workflow step reviewed; logic is a two-string compare (see §10) |
| AC6 | `AppVersionBadge.test.tsx` + picker/privacy tests | `v2.29.0` visible on both surfaces |
| AC7 | "renders nothing when the shell API is absent" | container empty after rejection settles |

## 8. Architecture / approach
Stamping lives in the release pipeline (option the issue prescribes): a prepare-step edit
committed by `@semantic-release/git`, so the tag itself carries the version — no
tag-time rewriting inside `release.yml`, and local checkouts of a release tag build
identical artifacts. Regex-targeted replacement (not `JSON.stringify`/TOML re-emit)
keeps diffs one-line and preserves formatting; a JSON parse after replacement verifies
correctness. Frontend reads the version from the bundle at runtime (`getVersion()`), never
from a constant, so it cannot drift. No network anywhere; offline-first untouched.

## 9. Slice breakdown
Single slice (~350 lines): script + tests, release wiring, workflow assert, badge + tests.

## 10. Risks / open questions
- The workflow assert can't be exercised by a unit test; it's a two-string comparison
  reviewed by eye and proven by the next release (manual-verify item).
- First release after merge rewrites `Cargo.lock`'s app version — `--locked` builds keep
  passing because the manifest version is stamped in the same commit.

## 11. References
#384, #381, #401 (VA runs), `semantic-release.yml`, `release.yml`,
`docs/architecture/offline-first-and-network-transparency.md` (no new egress).
