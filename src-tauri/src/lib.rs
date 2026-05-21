mod commands;
pub mod config;
mod correction;
pub mod db;
mod llm_client;
mod llm_settings;
mod lock_utils;
mod settings;
mod versioning;

use serde::Serialize;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::config::quality_filter::QualityFilterConfig;
use crate::correction::CorrectionEngine;
use crate::llm_client::CachedModels;
use crate::llm_settings::LlmSettings;
use crate::lock_utils::mutex_lock;
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
    correction_engine: Arc<CorrectionEngine>,
    init_status: Arc<AtomicU8>,
    init_error: Arc<RwLock<String>>,
    num_threads: Arc<AtomicU32>,
    settings: Arc<RwLock<VadSettings>>,
    llm_settings: Arc<RwLock<LlmSettings>>,
    quality_filter_config: Arc<RwLock<QualityFilterConfig>>,
    llm_models_cache: Arc<RwLock<Option<CachedModels>>>,
    selected_device: Arc<RwLock<Option<String>>>,
}

fn build_app_state(
    db: db::SpeechDatabase,
    llm_settings: LlmSettings,
    quality_filter_config: QualityFilterConfig,
) -> AppState {
    AppState {
        recording: Arc::new(AtomicBool::new(false)),
        stop_signal: Arc::new(AtomicBool::new(false)),
        db: Arc::new(Mutex::new(Some(db))),
        correction_engine: Arc::new(CorrectionEngine::new()),
        init_status: Arc::new(AtomicU8::new(0)),
        init_error: Arc::new(RwLock::new(String::new())),
        num_threads: Arc::new(AtomicU32::new(0)),
        settings: Arc::new(RwLock::new(VadSettings::default())),
        llm_settings: Arc::new(RwLock::new(llm_settings)),
        quality_filter_config: Arc::new(RwLock::new(quality_filter_config)),
        llm_models_cache: Arc::new(RwLock::new(None)),
        selected_device: Arc::new(RwLock::new(None)),
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

    let llm_settings = tauri::async_runtime::block_on(settings::load_llm_settings_from_db(&db));
    let quality_filter_config = tauri::async_runtime::block_on(settings::load_quality_filter_config_from_db(&db));
    let version_info = match tauri::async_runtime::block_on(versioning::AppVersionInfo::new(&db)) {
        Ok(info) => info,
        Err(err) => {
            error!("[version] failed to build version info: {err}");
            versioning::AppVersionInfo {
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                app_name: versioning::APP_NAME.to_string(),
                build_profile: if cfg!(debug_assertions) {
                    "debug".to_string()
                } else {
                    "release".to_string()
                },
                git_commit: None,
                schema_version: crate::db::schema::DB_SCHEMA_VERSION,
                config_schema_version: crate::config::quality_filter::QUALITY_FILTER_CONFIG_SCHEMA_VERSION,
                first_run_after_upgrade: false,
            }
        }
    };
    info!(
        "[version] app_version={} build_profile={} schema_version={} first_run_after_upgrade={}",
        version_info.app_version,
        version_info.build_profile,
        version_info.schema_version,
        version_info.first_run_after_upgrade
    );
    let state = build_app_state(db, llm_settings, quality_filter_config);
    {
        let db_guard = mutex_lock(&state.db);
        if let Some(db) = db_guard.as_ref() {
            let _ = tauri::async_runtime::block_on(commands::correction::reload_correction_rules(
                db,
                &state.correction_engine,
            ));
        }
    }

    // Remote-only client: recognition runs on the GB10 orchestrator.
    // Report ready immediately; connection errors surface at record time
    // (run_remote_session sets init_status=error with a message).
    state.init_status.store(1, Ordering::Relaxed);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .setup({
            let correction_engine = Arc::clone(&state.correction_engine);
            let db_state = Arc::clone(&state.db);
            move |app| {
                if let Some(icon) = app.default_window_icon().cloned() {
                    info!("[tray] creating tray icon");
                    TrayIconBuilder::new()
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

                let db_guard = mutex_lock(&db_state);
                if let Some(db) = db_guard.as_ref() {
                    let _ = tauri::async_runtime::block_on(commands::correction::reload_correction_rules(
                        db,
                        &correction_engine,
                    ));
                } else {
                    error!("[setup] database not initialized");
                }
                Ok(())
            }
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
            commands::export::copy_text_to_clipboard,
            commands::init::get_init_status,
            commands::settings::get_settings,
            commands::settings::apply_settings,
            commands::settings::list_llm_models,
            commands::settings::list_llm_models_with_url,
            commands::correction_api::list_correction_rules,
            commands::correction_api::create_correction_rule,
            commands::correction_api::update_correction_rule,
            commands::correction_api::delete_correction_rule,
            commands::correction_api::reload_correction_rules,
            commands::quality_filter::get_quality_filter_config,
            commands::quality_filter::save_quality_filter_config,
            commands::quality_filter::reset_quality_filter_config,
            commands::version::get_app_version_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
