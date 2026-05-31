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
//! **Binary delivery is deferred.** Placing the platform library under the
//! bundled `onnxruntime/` resource dir happens in the desktop installer
//! pipeline (`cargo tauri build`), which does not exist yet — see
//! `scripts/fetch-onnxruntime.sh` and the `onnxruntime` entry in
//! `tauri.conf.json`. Until then this resolver is a no-op in dev builds (the
//! library isn't present), and transcription falls back to a developer-set
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
    let candidate = resource_dir?.join("onnxruntime").join(ort_lib_filename());
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
}
