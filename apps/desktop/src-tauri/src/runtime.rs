//! Locating the native ONNX Runtime for audio transcription in a packaged app.
//!
//! The `transcribe` crate runs basic-pitch through ONNX Runtime via `ort`'s
//! `load-dynamic` backend, which `dlopen`s `libonnxruntime` at run time from
//! the `ORT_DYLIB_PATH` environment variable. In dev and CI that variable is
//! set by the developer / workflow. A *packaged* app has no such environment,
//! so at startup we resolve the runtime that ships alongside the app and set
//! `ORT_DYLIB_PATH` ourselves — the same "bundled resource" resolution we use
//! for `profiles/` (#112/#120).
//!
//! **Binary delivery (#383, PR #395):** the release workflow runs
//! `scripts/fetch-onnxruntime.sh` on every platform before `tauri build`
//! (macOS universal fetches BOTH arch dylibs under `onnxruntime/<arch>/`),
//! and a red pre-build assert keeps the resource from ever going missing
//! silently again. In dev builds the library usually isn't present and this
//! resolver is a no-op — transcription falls back to a developer-set
//! `ORT_DYLIB_PATH` or the system loader.

use std::path::{Path, PathBuf};

/// Platform-specific ONNX Runtime shared-library filename.
pub fn ort_lib_filename() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libonnxruntime.dylib"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "libonnxruntime.so"
    }
}

/// Decide what `ORT_DYLIB_PATH` should be set to, without touching the
/// environment (pure, so it is unit-testable).
///
/// Resolution order:
/// 1. If `ORT_DYLIB_PATH` is already set, respect it — return `None` (no
///    change). This is the dev/CI path and an explicit user override.
/// 2. Otherwise, if the bundled runtime exists at
///    `<resource_dir>/onnxruntime/<platform-lib>`, return that path.
/// 3. Otherwise return `None` — leave it to the system loader; transcription
///    surfaces a calm error if the runtime can't be found.
pub fn resolve_ort_dylib(resource_dir: Option<&Path>, env_already_set: bool) -> Option<PathBuf> {
    if env_already_set {
        return None;
    }
    let base = resource_dir?.join("onnxruntime");
    // #383: macOS universal builds bundle BOTH arch dylibs under arch
    // subdirs (an arm64 dylib dlopen'd by an x86_64 process is a
    // wrong-architecture failure, not a fallback). The running process's
    // arch picks; the flat path stays as the single-arch layout.
    let arch_candidate = base.join(std::env::consts::ARCH).join(ort_lib_filename());
    if arch_candidate.is_file() {
        return Some(arch_candidate);
    }
    let candidate = base.join(ort_lib_filename());
    candidate.is_file().then_some(candidate)
}

/// Point `ort` at the bundled ONNX Runtime by setting `ORT_DYLIB_PATH`, unless
/// it is already set. Call once at app startup, before any transcription.
pub fn configure_onnxruntime(app_handle: &tauri::AppHandle) {
    use tauri::Manager;

    let already_set = std::env::var_os("ORT_DYLIB_PATH").is_some();
    let resource_dir = app_handle.path().resource_dir().ok();
    match resolve_ort_dylib(resource_dir.as_deref(), already_set) {
        Some(path) => {
            // Startup is single-threaded; `ort` reads this lazily on the first
            // Session creation, which only happens later on a user action.
            std::env::set_var("ORT_DYLIB_PATH", &path);
            tracing::info!(path = %path.display(), "using bundled ONNX Runtime");
        }
        None if already_set => {
            tracing::debug!("ORT_DYLIB_PATH already set; leaving transcription runtime as-is");
        }
        None => {
            tracing::debug!(
                "no bundled ONNX Runtime found; audio transcription will rely on the system loader"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// OMR sidecar (PDF → MusicXML)
// ---------------------------------------------------------------------------

/// Platform-specific filename of the bundled OMR (oemer) sidecar executable.
pub fn omr_engine_filename() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "amc-omr-engine.exe"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "amc-omr-engine"
    }
}

/// Decide where the OMR sidecar binary lives, without touching the environment
/// (pure, so it is unit-testable). Mirrors [`resolve_ort_dylib`]:
///
/// 1. If `OMR_ENGINE_PATH` is already set, respect it — return `None` (no
///    change). This is the dev/CI path and an explicit override.
/// 2. Otherwise, if the bundled engine exists at
///    `<resource_dir>/omr/<platform-binary>`, return that path.
/// 3. Otherwise return `None` — the engine isn't bundled; PDF import surfaces a
///    calm "not available in this build" error instead of pretending.
pub fn resolve_omr_engine(resource_dir: Option<&Path>, env_already_set: bool) -> Option<PathBuf> {
    if env_already_set {
        return None;
    }
    let candidate = resource_dir?.join("omr").join(omr_engine_filename());
    candidate.is_file().then_some(candidate)
}

/// Point OMR at the bundled sidecar by setting `OMR_ENGINE_PATH`, unless it is
/// already set. Call once at app startup. A no-op when the engine isn't bundled
/// (dev builds): PDF import then reports it's unavailable rather than failing
/// obscurely.
pub fn configure_omr_engine(app_handle: &tauri::AppHandle) {
    use tauri::Manager;

    let already_set = std::env::var_os("OMR_ENGINE_PATH").is_some();
    let resource_dir = app_handle.path().resource_dir().ok();
    match resolve_omr_engine(resource_dir.as_deref(), already_set) {
        Some(path) => {
            std::env::set_var("OMR_ENGINE_PATH", &path);
            tracing::info!(path = %path.display(), "using bundled OMR engine");
        }
        None if already_set => {
            tracing::debug!("OMR_ENGINE_PATH already set; leaving PDF import engine as-is");
        }
        None => {
            tracing::debug!("no bundled OMR engine found; PDF score import will be unavailable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Unique temp dir per test invocation (avoids cross-test collisions).
    fn unique_dir(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("amc_ort_{tag}_{}_{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn respects_existing_env_even_when_bundled() {
        let base = unique_dir("env");
        let ort = base.join("onnxruntime");
        fs::create_dir_all(&ort).unwrap();
        fs::write(ort.join(ort_lib_filename()), b"stub").unwrap();
        // env already set → never override, regardless of a bundled lib.
        assert_eq!(resolve_ort_dylib(Some(&base), true), None);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn finds_bundled_runtime() {
        let base = unique_dir("bundled");
        let ort = base.join("onnxruntime");
        fs::create_dir_all(&ort).unwrap();
        let lib = ort.join(ort_lib_filename());
        fs::write(&lib, b"stub").unwrap();
        assert_eq!(resolve_ort_dylib(Some(&base), false), Some(lib));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn none_when_not_bundled_or_no_resource_dir() {
        let base = unique_dir("missing"); // never created
        assert_eq!(resolve_ort_dylib(Some(&base), false), None);
        assert_eq!(resolve_ort_dylib(None, false), None);
    }

    /// #383: a universal macOS bundle carries BOTH arch dylibs under arch
    /// subdirs; the running process's arch must win over the flat path —
    /// an arm64 dylib in an x86_64 process is a wrong-arch dlopen failure,
    /// not a fallback. Fails if the arch-subdir probe is dropped or probes
    /// after the flat path.
    #[test]
    fn arch_subdir_wins_over_flat_layout() {
        let base = unique_dir("arch");
        let ort = base.join("onnxruntime");
        let arch_dir = ort.join(std::env::consts::ARCH);
        fs::create_dir_all(&arch_dir).unwrap();
        // Both layouts present — the arch one must be chosen.
        fs::write(ort.join(ort_lib_filename()), b"flat").unwrap();
        let arch_lib = arch_dir.join(ort_lib_filename());
        fs::write(&arch_lib, b"arch").unwrap();
        assert_eq!(resolve_ort_dylib(Some(&base), false), Some(arch_lib));
        let _ = fs::remove_dir_all(&base);
    }

    /// A DIFFERENT arch's subdir must never satisfy resolution — only the
    /// flat layout can, and when neither matches the resolver stays None
    /// (calm error downstream, never a wrong-arch dlopen).
    #[test]
    fn foreign_arch_subdir_is_ignored() {
        let base = unique_dir("foreign");
        let other = if std::env::consts::ARCH == "aarch64" {
            "x86_64"
        } else {
            "aarch64"
        };
        let dir = base.join("onnxruntime").join(other);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(ort_lib_filename()), b"wrong-arch").unwrap();
        assert_eq!(resolve_ort_dylib(Some(&base), false), None);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn omr_respects_existing_env_even_when_bundled() {
        let base = unique_dir("omr_env");
        let dir = base.join("omr");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(omr_engine_filename()), b"stub").unwrap();
        assert_eq!(resolve_omr_engine(Some(&base), true), None);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn omr_finds_bundled_engine() {
        let base = unique_dir("omr_bundled");
        let dir = base.join("omr");
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join(omr_engine_filename());
        fs::write(&bin, b"stub").unwrap();
        assert_eq!(resolve_omr_engine(Some(&base), false), Some(bin));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn omr_none_when_not_bundled_or_no_resource_dir() {
        let base = unique_dir("omr_missing"); // never created
        assert_eq!(resolve_omr_engine(Some(&base), false), None);
        assert_eq!(resolve_omr_engine(None, false), None);
    }
}
