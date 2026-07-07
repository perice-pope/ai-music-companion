//! Tracing setup for the desktop shell: a file layer in the user's cache dir
//! (always) plus a stderr layer (debug builds), both behind one env filter.
//!
//! Lives in the lib so the filter's behavior is covered by `cargo test --lib`
//! — the CI desktop job does not run bin-target tests.

use std::fs::OpenOptions;
use std::io;
use std::sync::Mutex;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
// `Layer` as a trait is needed for `.boxed()`; imported anonymously so it
// doesn't clash with the `Layer` struct from `tracing_subscriber::fmt`.
use tracing_subscriber::Layer as _;

/// Filter applied when `RUST_LOG` is unset or blank. Workspace crates log at
/// `info` so the diagnostic breadcrumbs (score-follower install, first
/// score-position emit, pipeline lifecycle) reach the log testers pull —
/// `EnvFilter::from_default_env()` would silence everything below `error`
/// (#279/#313). Third-party crates stay at `warn` because the file layer
/// appends without rotation.
const DEFAULT_LOG_DIRECTIVES: &str = "warn,ai_music_companion=info,brain=info,ears=info,\
     groove=info,idiom=info,omr=info,theory=info,tone=info,transcribe=info,variations=info";

fn env_filter(rust_log: Option<&str>) -> EnvFilter {
    match rust_log {
        Some(directives) if !directives.trim().is_empty() => EnvFilter::new(directives),
        _ => EnvFilter::new(DEFAULT_LOG_DIRECTIVES),
    }
}

/// Installs the global tracing subscriber. Call once, before anything logs.
pub fn init() {
    // Determine log file path. `dirs::cache_dir()` returns an
    // `Option<PathBuf>`; fall back to `/tmp` on platforms without one.
    let log_dir = match dirs::cache_dir() {
        Some(cache_dir) => cache_dir
            .join("ai-music-companion")
            .to_string_lossy()
            .into_owned(),
        None => "/tmp".to_string(),
    };

    // Create log directory if it doesn't exist
    let _ = std::fs::create_dir_all(&log_dir);

    // Try to open log file for writing (append mode)
    let log_file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{}/app.log", log_dir))
    {
        Ok(file) => Some(file),
        Err(e) => {
            eprintln!("failed to open log file: {}", e);
            None
        }
    };

    // Build file layer if log file was successfully opened.
    // `tracing_subscriber::MakeWriter` is implemented for `Mutex<W: Write>`
    // but not for a bare `LineWriter<File>`, so wrap in a Mutex.
    let file_layer = log_file.map(|file| {
        Layer::new()
            .with_writer(Mutex::new(io::LineWriter::new(file)))
            .with_ansi(false)
            .boxed()
    });

    // Build console layer for debug builds
    let console_layer = if cfg!(debug_assertions) {
        Some(Layer::new().with_writer(io::stderr).with_ansi(true).boxed())
    } else {
        None
    };

    // Combine layers. The filter MUST come from `env_filter` — constructing
    // it any other way (e.g. `EnvFilter::from_default_env()`) re-introduces
    // the #313 error-only default the tests below guard against.
    tracing_subscriber::registry()
        .with(env_filter(std::env::var("RUST_LOG").ok().as_deref()))
        .with(file_layer)
        .with(console_layer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Runs `emit` under a subscriber using `filter` and returns what the
    /// fmt layer wrote — i.e. what would land in the log file.
    fn captured(filter: EnvFilter, emit: impl FnOnce()) -> String {
        let writer = CaptureWriter::default();
        let subscriber = tracing_subscriber::registry().with(filter).with(
            Layer::new()
                .with_writer(writer.clone())
                .with_ansi(false)
                .boxed(),
        );
        tracing::subscriber::with_default(subscriber, emit);
        let bytes = writer.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    // The #313 finding: with RUST_LOG unset, the tester's log had no
    // follower/score-position entries because the old default filter
    // dropped everything below `error`.
    #[test]
    fn default_filter_keeps_workspace_diagnostic_breadcrumbs() {
        let out = captured(env_filter(None), || {
            tracing::info!("score follower installed");
            tracing::warn!("score session started WITHOUT a follower");
        });
        assert!(
            out.contains("score follower installed"),
            "info breadcrumb filtered out of the log: {out:?}"
        );
        assert!(out.contains("score session started WITHOUT a follower"));
    }

    // Breadcrumbs added to the analysis crates must land too — each
    // workspace member needs its own directive, not just the app crate.
    #[test]
    fn workspace_crate_info_passes_the_default_filter() {
        let out = captured(env_filter(None), || {
            tracing::info!(target: "brain::follower", "follower aligned");
            tracing::info!(target: "ears::pitch", "detector reconfigured");
        });
        assert!(
            out.contains("follower aligned"),
            "brain info filtered: {out:?}"
        );
        assert!(
            out.contains("detector reconfigured"),
            "ears info filtered: {out:?}"
        );
    }

    #[test]
    fn explicit_rust_log_still_wins_over_the_default() {
        let out = captured(env_filter(Some("error")), || {
            tracing::info!("info suppressed by explicit RUST_LOG");
            tracing::error!("error kept by explicit RUST_LOG");
        });
        assert!(!out.contains("info suppressed by explicit RUST_LOG"));
        assert!(out.contains("error kept by explicit RUST_LOG"));
    }

    #[test]
    fn blank_rust_log_gets_the_default_not_error_only() {
        let out = captured(env_filter(Some("  ")), || {
            tracing::info!("breadcrumb with blank RUST_LOG");
        });
        assert!(
            out.contains("breadcrumb with blank RUST_LOG"),
            "blank RUST_LOG must fall back to the workspace default: {out:?}"
        );
    }

    #[test]
    fn third_party_info_stays_out_of_the_unrotated_log() {
        let out = captured(env_filter(None), || {
            tracing::info!(target: "wry::webview", "chatty dependency chatter");
            tracing::info!(target: "hyper::client", "more dependency chatter");
            tracing::warn!(target: "wry::webview", "dependency warning");
        });
        // The warn floor is global for third parties, not a per-crate carve-out.
        assert!(!out.contains("chatty dependency chatter"));
        assert!(!out.contains("more dependency chatter"));
        assert!(out.contains("dependency warning"));
    }
}
