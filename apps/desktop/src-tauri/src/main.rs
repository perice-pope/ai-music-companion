// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_music_companion::commands::AppState;
use std::fs::OpenOptions;
use std::io;
use std::sync::Mutex;
// `Manager` brings `App::manage` / `App::handle` into scope for the setup hook.
use tauri::Manager;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
// `Layer` as a trait is needed for `.boxed()`; imported anonymously so it
// doesn't clash with the `Layer` struct from `tracing_subscriber::fmt`.
use tracing_subscriber::Layer as _;

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

fn init_tracing() {
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

    // Combine layers
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(file_layer)
        .with(console_layer)
        .init();
}

fn main() {
    init_tracing();

    tauri::Builder::default()
        .setup(|app| {
            // Initialise AppState inside setup so the AppHandle is available
            // for resolving bundled `profiles/` in packaged installs (#112).
            app.manage(AppState::new_with_app_handle(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            ai_music_companion::commands::start_practice_session,
            ai_music_companion::commands::switch_instrument,
            ai_music_companion::commands::end_practice_session,
            ai_music_companion::commands::list_instruments,
            ai_music_companion::commands::get_session_history,
            ai_music_companion::commands::get_session_detail,
            ai_music_companion::commands::get_practice_stats,
            ai_music_companion::commands::import_score,
            ai_music_companion::commands::import_midi_file,
            ai_music_companion::commands::import_audio_file,
            ai_music_companion::commands::list_scores,
            ai_music_companion::commands::get_score,
            ai_music_companion::commands::delete_score,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(ping(), "pong");
    }
}
