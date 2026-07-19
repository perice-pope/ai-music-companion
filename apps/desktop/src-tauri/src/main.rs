// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_music_companion::commands::AppState;
// `Manager` brings `App::manage` / `App::handle` into scope for the setup hook.
use tauri::Manager;

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

fn main() {
    ai_music_companion::logging::init();

    let mut builder = tauri::Builder::default();

    // Register the Tauri v2 auto-updater (desktop only). This makes the
    // updater commands available to the frontend but performs NO network I/O
    // on its own — an update check happens only when the UI explicitly calls
    // it. The endpoint + pubkey live in `tauri.conf.json` under
    // `plugins.updater`. See docs/release.md for the signing-key workflow.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
        // #58 E1: the updater's official pairing — "Restart now" relaunches
        // into the staged build. No network, no other capability.
        builder = builder.plugin(tauri_plugin_process::init());
    }

    builder
        .setup(|app| {
            // Point transcription at the bundled ONNX Runtime (if present)
            // before any audio import can run. No-op when ORT_DYLIB_PATH is
            // already set (dev/CI) or the runtime isn't bundled yet.
            ai_music_companion::runtime::configure_onnxruntime(app.handle());
            // Likewise point PDF→sheet-music import at the bundled OMR engine
            // (if present) by setting OMR_ENGINE_PATH. No-op when unset/unbundled
            // — PDF import then reports it's unavailable rather than failing
            // obscurely.
            ai_music_companion::runtime::configure_omr_engine(app.handle());
            // Initialise AppState inside setup so the AppHandle is available
            // for resolving bundled `profiles/` in packaged installs (#112).
            app.manage(AppState::new_with_app_handle(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            ai_music_companion::commands::start_practice_session,
            ai_music_companion::commands::frontend_breadcrumb,
            ai_music_companion::commands::switch_instrument,
            ai_music_companion::commands::end_practice_session,
            ai_music_companion::commands::start_accompaniment,
            ai_music_companion::commands::start_pocket,
            ai_music_companion::commands::stop_pocket,
            ai_music_companion::commands::set_pocket_tempo,
            ai_music_companion::commands::set_band_tempo,
            ai_music_companion::commands::stop_accompaniment,
            ai_music_companion::commands::set_accompaniment_key,
            ai_music_companion::commands::clear_accompaniment_key,
            ai_music_companion::commands::list_instruments,
            ai_music_companion::commands::get_app_capabilities,
            ai_music_companion::commands::get_coaching_tip,
            ai_music_companion::commands::record_coaching_tip,
            ai_music_companion::commands::get_reveal,
            ai_music_companion::commands::record_reveal,
            ai_music_companion::commands::start_lesson,
            ai_music_companion::commands::submit_drill,
            ai_music_companion::commands::end_lesson,
            ai_music_companion::commands::start_explore_variation,
            ai_music_companion::commands::apply_variation_delta,
            ai_music_companion::commands::edit_explore_note,
            ai_music_companion::commands::explore_last_phrase,
            ai_music_companion::commands::undo_explore_edit,
            ai_music_companion::commands::end_explore,
            ai_music_companion::commands::get_learner_model_blob,
            ai_music_companion::commands::get_mastery_wheel,
            ai_music_companion::commands::get_sound_mirror,
            ai_music_companion::commands::get_session_history,
            ai_music_companion::commands::get_session_detail,
            ai_music_companion::commands::get_practice_stats,
            ai_music_companion::commands::get_taste_profile,
            ai_music_companion::commands::set_taste_profile,
            ai_music_companion::commands::import_score,
            ai_music_companion::commands::import_midi_file,
            ai_music_companion::commands::list_midi_parts,
            ai_music_companion::commands::explore_measure,
            ai_music_companion::commands::preview_opener,
            ai_music_companion::commands::begin_opener,
            ai_music_companion::commands::save_opener_recipe,
            ai_music_companion::commands::list_opener_recipes,
            ai_music_companion::commands::delete_opener_recipe,
            ai_music_companion::commands::recall_last_opener,
            ai_music_companion::commands::begin_opener_recall,
            ai_music_companion::commands::explore_chord,
            ai_music_companion::commands::explore_progression,
            ai_music_companion::commands::session_chord_chart,
            ai_music_companion::commands::list_score_parts,
            ai_music_companion::commands::import_musicxml_file,
            ai_music_companion::commands::import_audio_file,
            ai_music_companion::commands::recognize_pdf_score,
            ai_music_companion::commands::list_scores,
            ai_music_companion::commands::get_score,
            ai_music_companion::commands::check_piece_match,
            ai_music_companion::commands::my_patterns,
            ai_music_companion::commands::practice_suggestions,
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
