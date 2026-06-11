//! The shipped OMR engine: a bundled, frozen `oemer` binary invoked
//! out-of-process.
//!
//! **Contract with the sidecar binary:** it reads **PDF bytes on stdin** and
//! writes **MusicXML on stdout**; a non-zero exit means failure and stderr
//! carries the reason. Running recognition as a child process keeps the
//! Python-derived engine out of our address space and needs **no network** —
//! the binary is bundled at build time (`scripts/fetch-omr-engine.sh`), so the
//! feature works fully offline.
//!
//! The PDF is streamed to the child on a separate thread while we read its
//! stdout, so a large input can't deadlock on a full pipe buffer.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::{OmrEngine, OmrError};

/// Invokes a bundled `oemer` sidecar binary to recognize a PDF into MusicXML.
pub struct SidecarOmrEngine {
    binary: PathBuf,
}

impl SidecarOmrEngine {
    /// Build an engine that runs the sidecar executable at `binary`.
    ///
    /// The desktop app resolves this path from the Tauri resource dir (or an
    /// `OMR_ENGINE_PATH` override) at startup — see the app's `runtime` module,
    /// which mirrors how the ONNX Runtime is located.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

impl OmrEngine for SidecarOmrEngine {
    fn recognize_pdf(&self, pdf: &[u8]) -> Result<String, OmrError> {
        let mut child = Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                OmrError::EngineUnavailable(format!(
                    "could not launch {}: {e}",
                    self.binary.display()
                ))
            })?;

        // Stream the PDF to the child's stdin on its own thread, then let it
        // close (EOF) — concurrent with reading stdout to avoid a pipe-buffer
        // deadlock on large documents.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| OmrError::Engine("sidecar stdin was unavailable".to_string()))?;
        let pdf_owned = pdf.to_vec();
        let writer = std::thread::spawn(move || stdin.write_all(&pdf_owned));

        let output = child
            .wait_with_output()
            .map_err(|e| OmrError::Engine(format!("sidecar did not complete: {e}")))?;

        // Surface a stdin write failure rather than swallow it — except a
        // `BrokenPipe`, which just means the sidecar finished (and closed its
        // stdin) before reading the whole PDF. That's benign, like piping into
        // `head`: the real verdict is the exit status and stdout below.
        match writer.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
            Ok(Err(e)) => return Err(OmrError::Engine(format!("writing PDF to sidecar failed: {e}"))),
            Err(_) => return Err(OmrError::Engine("sidecar input thread panicked".to_string())),
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OmrError::Engine(format!(
                "sidecar exited with {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        String::from_utf8(output.stdout)
            .map_err(|_| OmrError::Engine("sidecar produced non-UTF-8 output".to_string()))
    }
}

// The subprocess contract is exercised with tiny shell-script stand-ins for the
// real engine. Unix-only because the scripts use a POSIX shell; CI and the
// founder's Mac are both Unix.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Write an executable shell script to a unique temp path and return it.
    fn script(tag: &str, body: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("amc_omr_{tag}_{}_{n}.sh", std::process::id()));
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    fn reads_stdin_and_returns_stdout_musicxml() {
        // Drains stdin, then prints a fixed score to stdout.
        let bin = script(
            "ok",
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '<score-partwise><part id=\"P1\"><measure number=\"1\"/></part></score-partwise>'\n",
        );
        let engine = SidecarOmrEngine::new(&bin);
        let xml = engine.recognize_pdf(b"%PDF-1.7 data").expect("engine succeeds");
        assert!(xml.contains("<score-partwise>"));
        let _ = fs::remove_file(&bin);
    }

    #[test]
    fn nonzero_exit_surfaces_stderr_as_engine_error() {
        let bin = script(
            "fail",
            "#!/bin/sh\ncat >/dev/null\necho 'page too blurry' 1>&2\nexit 3\n",
        );
        let engine = SidecarOmrEngine::new(&bin);
        let err = engine.recognize_pdf(b"%PDF-1.7").expect_err("engine fails");
        match err {
            OmrError::Engine(msg) => assert!(msg.contains("page too blurry"), "got: {msg}"),
            other => panic!("expected Engine error, got {other:?}"),
        }
        let _ = fs::remove_file(&bin);
    }

    #[test]
    fn missing_binary_reports_engine_unavailable() {
        let engine = SidecarOmrEngine::new("/nonexistent/amc-omr-engine-xyz");
        let err = engine.recognize_pdf(b"%PDF-1.7").expect_err("no binary");
        assert!(matches!(err, OmrError::EngineUnavailable(_)));
    }

    #[test]
    fn large_input_does_not_deadlock() {
        // A sidecar that ignores stdin and returns immediately must not hang
        // even when we feed it more than a pipe buffer's worth of PDF bytes.
        let bin = script(
            "big",
            "#!/bin/sh\nprintf '%s' '<score-partwise></score-partwise>'\n",
        );
        let engine = SidecarOmrEngine::new(&bin);
        let big = vec![b'%'; 1 << 20]; // 1 MiB
        let xml = engine.recognize_pdf(&big).expect("no deadlock");
        assert!(xml.contains("<score-partwise>"));
        let _ = fs::remove_file(&bin);
    }
}
