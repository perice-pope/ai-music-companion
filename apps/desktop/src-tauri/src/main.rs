// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_music_companion::commands::AppState;
use std::fs::OpenOptions;
use std::io;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

fn init_tracing() {
    // Determine log file path
    let log_dir = if let Some(cache_dir) = dirs::cache_dir().to_str() {
        format!("{}/ai-music-companion", cache_dir)
    } else {
        "/tmp".to_string()
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

    // Build file layer if log file was successfully opened
    let file_layer = log_file
        .map(|file| {
            Layer::new()
                .with_writer(io::LineWriter::new(file))
                .with_ansi(false)
                .boxed()
        })
        .boxed();

    // Build console layer for debug builds
    let console_layer = if cfg!(debug_assertions) {
        Some(
            Layer::new()
                .with_writer(io::stderr)
                .with_ansi(true)
                .boxed(),
        )
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
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            ping,
            ai_music_companion::commands::start_practice_session,
            ai_music_companion::commands::switch_instrument,
            ai_music_companion::commands::end_practice_session,
            ai_music_companion::commands::list_instruments,
            ai_music_companion::commands::get_session_history,
            ai_music_companion::commands::get_session_detail,
            ai_music_companion::commands::get_practice_stats,
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
