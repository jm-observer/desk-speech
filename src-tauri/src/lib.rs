mod audio_buffer;
mod commands;
pub mod config;
mod correction;
pub mod db;
mod llm_client;
mod llm_settings;
mod lock_utils;
mod settings;
mod versioning;

use audio_buffer::RollingAudioBuffer;
use serde::Serialize;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use crate::config::quality_filter::QualityFilterConfig;
use crate::correction::CorrectionEngine;
use crate::llm_client::CachedModels;
use crate::llm_settings::LlmSettings;
use crate::lock_utils::{mutex_lock, read_lock, write_lock};
use crate::settings::VadSettings;
use chrono::Local;
use log::{error, info, warn};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use super::{remove_segment_from_memory, SegmentResult};
    use crate::lock_utils::read_lock;

    fn seg(start: f32, end: f32, text: &str, revision: i64) -> SegmentResult {
        SegmentResult {
            segment_id: 1,
            revision,
            start,
            end,
            wall_start: "2026-01-01 00:00:00".to_string(),
            wall_end: "2026-01-01 00:00:01".to_string(),
            text: text.to_string(),
            text_optimized: Some("opt".to_string()),
            text_english: Some("en".to_string()),
            optimize_status: "success".to_string(),
            translate_status: "success".to_string(),
            finalization_check_state: "not_ready".to_string(),
            is_discarded: false,
            discard_reason: None,
            discard_source: None,
            discard_confidence: None,
            quality_check_status: "pending".to_string(),
            text_raw: text.to_string(),
        }
    }

    #[test]
    fn remove_segment_from_memory_removes_matching_segment_id_only() {
        let segments = Arc::new(RwLock::new(vec![
            seg(0.0, 1.0, "第一句", 1),
            seg(1.0, 2.0, "第二句", 2),
        ]));
        {
            let mut guard = segments.write().expect("lock segments");
            guard[0].segment_id = 100;
            guard[1].segment_id = 200;
        }

        let removed = remove_segment_from_memory(&segments, 100);

        assert!(removed);
        let guard = read_lock(&segments);
        assert_eq!(guard.len(), 1);
        assert_eq!(guard[0].segment_id, 200);
        assert_eq!(guard[0].text, "第二句");
    }

    #[test]
    fn remove_segment_from_memory_returns_false_when_segment_missing() {
        let segments = Arc::new(RwLock::new(vec![seg(0.0, 1.0, "第一句", 1)]));

        let removed = remove_segment_from_memory(&segments, 999);

        assert!(!removed);
        let guard = read_lock(&segments);
        assert_eq!(guard.len(), 1);
        assert_eq!(guard[0].segment_id, 1);
    }
}

#[derive(Serialize, Clone)]
pub(crate) struct SegmentResult {
    segment_id: u64,
    revision: i64,
    start: f32,
    end: f32,
    wall_start: String,
    wall_end: String,
    text: String,
    text_optimized: Option<String>,
    text_english: Option<String>,
    optimize_status: String,
    translate_status: String,
    finalization_check_state: String,
    // Plan 2: discard judgment fields
    is_discarded: bool,
    discard_reason: Option<String>,
    discard_source: Option<String>,
    discard_confidence: Option<f32>,
    quality_check_status: String,
    // Raw ASR text for judgment
    text_raw: String,
}

pub(crate) fn update_segment_llm_state(
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    revision: i64,
    optimize_status: Option<&str>,
    translate_status: Option<&str>,
    optimized: Option<String>,
    english: Option<String>,
) {
    let mut segs = write_lock(segments);
    if let Some(seg) = segs.iter_mut().rev().find(|seg| seg.revision == revision) {
        if let Some(status) = optimize_status {
            seg.optimize_status = status.to_string();
        }
        if let Some(status) = translate_status {
            seg.translate_status = status.to_string();
        }
        if let Some(text) = optimized {
            seg.text_optimized = Some(text);
        }
        if let Some(text) = english {
            seg.text_english = Some(text);
        }
    }
}

pub(crate) fn remove_segment_from_memory(segments: &Arc<RwLock<Vec<SegmentResult>>>, segment_id: u64) -> bool {
    let mut segs = write_lock(segments);
    let original_len = segs.len();
    segs.retain(|seg| seg.segment_id != segment_id);
    segs.len() != original_len
}

#[derive(Serialize, Clone)]
pub(crate) struct RecordingState {
    recording: bool,
    segments: Vec<SegmentResult>,
    elapsed_secs: f32,
    audio_window_start_sec: f32,
    audio_window_end_sec: f32,
}

pub(crate) struct AppState {
    recording: Arc<AtomicBool>,
    stop_signal: Arc<AtomicBool>,
    segments: Arc<RwLock<Vec<SegmentResult>>>,
    recorded_audio: Arc<RwLock<RollingAudioBuffer>>,
    db: Arc<Mutex<Option<db::SpeechDatabase>>>,
    correction_engine: Arc<CorrectionEngine>,
    start_wall_clock: Arc<RwLock<Option<chrono::DateTime<Local>>>>,
    start_instant: Arc<RwLock<Option<Instant>>>,
    init_status: Arc<AtomicU8>,
    init_error: Arc<RwLock<String>>,
    num_threads: Arc<AtomicU32>,
    settings: Arc<RwLock<VadSettings>>,
    llm_settings: Arc<RwLock<LlmSettings>>,
    quality_filter_config: Arc<RwLock<QualityFilterConfig>>,
    llm_models_cache: Arc<RwLock<Option<CachedModels>>>,
    selected_device: Arc<RwLock<Option<String>>>,
}

// ---------------------------------------------------------------------------
// cpal microphone capture
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Device enumeration
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Resource directory & model init
// ---------------------------------------------------------------------------

fn build_app_state(
    db: db::SpeechDatabase,
    llm_settings: LlmSettings,
    quality_filter_config: QualityFilterConfig,
) -> AppState {
    AppState {
        recording: Arc::new(AtomicBool::new(false)),
        stop_signal: Arc::new(AtomicBool::new(false)),
        segments: Arc::new(RwLock::new(Vec::new())),
        recorded_audio: Arc::new(RwLock::new(RollingAudioBuffer::new())),
        db: Arc::new(Mutex::new(Some(db))),
        correction_engine: Arc::new(CorrectionEngine::new()),
        start_wall_clock: Arc::new(RwLock::new(None)),
        start_instant: Arc::new(RwLock::new(None)),
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
            commands::history_api::list_segments,
            commands::history_api::tail_segments,
            commands::history_api::delete_segment,
            commands::export::save_segment_as_wav,
            commands::export::save_all_audio,
            commands::export::get_recorded_audio_path,
            commands::export::export_srt,
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
            commands::manual_optimize::manual_optimize_translate,
            commands::quality_filter::get_quality_filter_config,
            commands::quality_filter::save_quality_filter_config,
            commands::quality_filter::reset_quality_filter_config,
            commands::version::get_app_version_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
