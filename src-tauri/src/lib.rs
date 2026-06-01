mod commands;
pub mod db;
mod llm_settings;
mod lock_utils;
mod settings;

use serde::Serialize;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::llm_settings::LlmSettings;
use crate::settings::VadSettings;
use log::{error, info, warn};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

#[derive(Serialize, Clone)]
pub(crate) struct RecordingState {
    recording: bool,
}

pub(crate) struct AppState {
    recording: Arc<AtomicBool>,
    stop_signal: Arc<AtomicBool>,
    db: Arc<Mutex<Option<db::SpeechDatabase>>>,
    init_status: Arc<AtomicU8>,
    init_error: Arc<RwLock<String>>,
    settings: Arc<RwLock<VadSettings>>,
    llm_settings: Arc<RwLock<LlmSettings>>,
    selected_device: Arc<RwLock<Option<String>>>,
    /// Currently selected remote orchestrator WS URL (e.g.
    /// `ws://192.168.0.68:8090/stream`). Replaces the old `REMOTE_ASR_URL` env
    /// var — edited from the desktop UI, persisted in SQLite as `remote.url`.
    remote_url: Arc<RwLock<String>>,
    /// User-added custom URLs surfaced in the connection dropdown (in addition
    /// to the built-in default). Persisted as JSON in SQLite under `remote.url_presets`.
    remote_url_presets: Arc<RwLock<Vec<String>>>,
}

fn build_app_state(
    db: db::SpeechDatabase,
    vad_settings: VadSettings,
    llm_settings: LlmSettings,
    remote_url: String,
    remote_url_presets: Vec<String>,
) -> AppState {
    AppState {
        recording: Arc::new(AtomicBool::new(false)),
        stop_signal: Arc::new(AtomicBool::new(false)),
        db: Arc::new(Mutex::new(Some(db))),
        init_status: Arc::new(AtomicU8::new(0)),
        init_error: Arc::new(RwLock::new(String::new())),
        settings: Arc::new(RwLock::new(vad_settings)),
        llm_settings: Arc::new(RwLock::new(llm_settings)),
        selected_device: Arc::new(RwLock::new(None)),
        remote_url: Arc::new(RwLock::new(remote_url)),
        remote_url_presets: Arc::new(RwLock::new(remote_url_presets)),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let workspace = match custom_utils::args::workspace(&None, "streaming-speech") {
        Ok(ws) => ws,
        Err(err) => {
            error!("[init] failed to determine workspace path: {err}");
            return;
        }
    };
    let db_path = workspace.join("speech_history.db");
    if let Err(err) = std::fs::create_dir_all(&workspace) {
        error!("[db] cannot create parent dir {}: {err}", workspace.display());
        return;
    }
    let db = match tauri::async_runtime::block_on(db::SpeechDatabase::init(&db_path)) {
        Ok(db) => db,
        Err(err) => {
            error!("[db] init failed at {}: {err}", db_path.display());
            return;
        }
    };

    let vad_settings = tauri::async_runtime::block_on(settings::load_vad_settings_from_db(&db));
    let llm_settings = tauri::async_runtime::block_on(settings::load_llm_settings_from_db(&db));
    let (remote_url, remote_url_presets) =
        tauri::async_runtime::block_on(settings::load_remote_settings_from_db(&db));
    let state = build_app_state(db, vad_settings, llm_settings, remote_url, remote_url_presets);

    // Remote-only client: recognition runs on the GB10 orchestrator.
    // Report ready immediately; connection errors surface at record time
    // (run_remote_session sets init_status=error with a message).
    state.init_status.store(1, Ordering::Relaxed);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            if let Some(icon) = app.default_window_icon().cloned() {
                info!("[tray] creating tray icon");
                // ID `main` lets `app.tray_by_id("main")` retrieve the handle
                // later — used by `commands::notify::bounce_tray_twice` to
                // animate the tray icon when a segment finishes optimize + translate.
                TrayIconBuilder::with_id("main")
                    .icon(icon)
                    .tooltip("StreamSpeech")
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            info!("[tray] left click received, restoring window");
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                if let Err(err) = window.show() {
                                    error!("show window from tray failed: {}", err);
                                    return;
                                }
                                info!("[tray] window shown from tray click");
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                                info!("[tray] window focus requested after tray click");
                            } else {
                                warn!("[tray] main window not found on tray click");
                            }
                        }
                    })
                    .build(app)?;
                info!("[tray] tray icon created");
            } else {
                warn!("[setup] default window icon missing, tray icon not created");
            }
            Ok(())
        })
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::device::list_input_devices,
            commands::device::set_input_device,
            commands::device::get_selected_device,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::clear_results,
            commands::recording::get_recording_state,
            commands::remote::fetch_remote_history,
            commands::export::copy_text_to_clipboard,
            commands::init::get_init_status,
            commands::settings::get_settings,
            commands::settings::apply_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
