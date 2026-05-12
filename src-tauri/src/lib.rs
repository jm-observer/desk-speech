mod audio_buffer;
mod commands;
mod config;
mod correction;
pub mod db;
mod db_worker;
mod llm_client;
mod llm_settings;
mod lock_utils;
mod model_registry;
mod settings;

use audio_buffer::RollingAudioBuffer;
use model_registry::get_model_config;
use serde::Serialize;
use sherpa_onnx::{OfflineRecognizer, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use crate::config::quality_filter::QualityFilterConfig;
use crate::correction::CorrectionEngine;
use crate::llm_client::CachedModels;
use crate::llm_settings::LlmSettings;
use crate::lock_utils::{mutex_lock, read_lock, write_lock};
use crate::settings::VadSettings;
use chrono::Local;
use log::{debug, error, info, warn};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

const MERGE_MAX_GAP_SEC: f32 = 5.6;
pub(crate) const FINALIZE_SILENCE_MS: u64 = 10_000;

pub(crate) fn merge_segment_in_place(segments: &mut [SegmentResult], incoming: &SegmentResult) -> bool {
    let Some(last) = segments.last_mut() else {
        return false;
    };

    let gap_sec = incoming.start - last.end;
    if !(0.0..=MERGE_MAX_GAP_SEC).contains(&gap_sec) {
        return false;
    }

    last.end = incoming.end;
    last.revision = incoming.revision;
    last.wall_end = incoming.wall_end.clone();
    last.text = format!("{} {}", last.text, incoming.text).trim().to_string();
    // Merged transcript invalidates previous LLM output.
    last.text_optimized = None;
    last.text_english = None;
    last.optimize_status = "pending".to_string();
    last.translate_status = "blocked".to_string();
    last.finalization_check_state = "not_ready".to_string();
    last.update_type = SegmentUpdateType::Replace;
    true
}

#[cfg(test)]
mod tests {
    use super::{merge_segment_in_place, SegmentResult, SegmentUpdateType};

    fn seg(start: f32, end: f32, text: &str, revision: i64) -> SegmentResult {
        SegmentResult {
            segment_id: 1,
            revision,
            update_type: SegmentUpdateType::Append,
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
    fn merge_keeps_working_for_long_running_sentence_when_gap_is_small() {
        let mut segments = vec![seg(0.0, 29.0, "前半段", 1)];
        let incoming = seg(29.2, 31.0, "后半段", 2);
        let merged = merge_segment_in_place(&mut segments, &incoming);

        assert!(merged);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "前半段 后半段");
        assert_eq!(segments[0].end, 31.0);
        assert_eq!(segments[0].revision, 2);
        assert!(segments[0].text_optimized.is_none());
        assert!(segments[0].text_english.is_none());
        assert_eq!(segments[0].optimize_status, "pending");
        assert_eq!(segments[0].translate_status, "blocked");
    }

    #[test]
    fn merge_rejects_when_gap_exceeds_threshold() {
        let mut segments = vec![seg(0.0, 5.0, "第一句", 1)];
        let incoming = seg(10.7, 11.0, "第二句", 2);
        let merged = merge_segment_in_place(&mut segments, &incoming);

        assert!(!merged);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "第一句");
    }
}

/// Which ASR model to bundle. Build scripts patch these via sed.
const MODEL_TYPE: u32 = 15;
const MODEL_NAME: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17";

#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SegmentUpdateType {
    Append,
    Replace,
}

#[derive(Serialize, Clone)]
pub(crate) struct SegmentResult {
    segment_id: u64,
    revision: i64,
    update_type: SegmentUpdateType,
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

pub(crate) fn set_segment_finalization_state(segments: &Arc<RwLock<Vec<SegmentResult>>>, revision: i64, state: &str) {
    let mut segs = write_lock(segments);
    if let Some(seg) = segs.iter_mut().rev().find(|seg| seg.revision == revision) {
        seg.finalization_check_state = state.to_string();
    }
}

pub(crate) fn set_segment_discard_state(
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    revision: i64,
    is_discarded: bool,
    discard_reason: Option<String>,
    discard_source: Option<String>,
    discard_confidence: Option<f32>,
    quality_check_status: &str,
) {
    let mut segs = write_lock(segments);
    if let Some(seg) = segs.iter_mut().rev().find(|seg| seg.revision == revision) {
        seg.is_discarded = is_discarded;
        seg.discard_reason = discard_reason;
        seg.discard_source = discard_source;
        seg.discard_confidence = discard_confidence;
        seg.quality_check_status = quality_check_status.to_string();
    }
}

pub(crate) fn can_start_finalization_check(segments: &Arc<RwLock<Vec<SegmentResult>>>, revision: i64) -> bool {
    let segs = read_lock(segments);
    let Some(seg) = segs.iter().rev().find(|seg| seg.revision == revision) else {
        return false;
    };
    if seg.text.trim().is_empty() {
        return false;
    }
    if matches!(seg.optimize_status.as_str(), "pending" | "running") {
        return false;
    }
    if matches!(seg.translate_status.as_str(), "pending" | "running") {
        return false;
    }
    seg.finalization_check_state == "not_ready"
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
    recognizer: Arc<RwLock<Option<OfflineRecognizer>>>,
    vad: Arc<RwLock<Option<VoiceActivityDetector>>>,
    recording: Arc<AtomicBool>,
    stop_signal: Arc<AtomicBool>,
    segments: Arc<RwLock<Vec<SegmentResult>>>,
    recorded_audio: Arc<RwLock<RollingAudioBuffer>>,
    db: Arc<Mutex<Option<db::SpeechDatabase>>>,
    db_writer: Arc<SyncSender<db_worker::DbEvent>>,
    correction_engine: Arc<CorrectionEngine>,
    start_wall_clock: Arc<RwLock<Option<chrono::DateTime<Local>>>>,
    start_instant: Arc<RwLock<Option<Instant>>>,
    init_status: Arc<AtomicU8>,
    init_error: Arc<RwLock<String>>,
    num_threads: Arc<AtomicU32>,
    next_realtime_segment_id: Arc<AtomicU64>,
    next_revision: Arc<AtomicU64>,
    settings: Arc<RwLock<VadSettings>>,
    llm_settings: Arc<RwLock<LlmSettings>>,
    quality_filter_config: Arc<RwLock<QualityFilterConfig>>,
    llm_models_cache: Arc<RwLock<Option<CachedModels>>>,
    selected_device: Arc<RwLock<Option<String>>>,
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
}

pub(crate) struct RecordingAnchor {
    base_wall: chrono::DateTime<Local>,
    audio_offset: u64,
}

pub(crate) struct RecordingRuntime<'a> {
    stop_signal: &'a AtomicBool,
    segments: &'a Arc<RwLock<Vec<SegmentResult>>>,
    next_realtime_segment_id: Arc<AtomicU64>,
    next_revision: Arc<AtomicU64>,
    correction_engine: &'a CorrectionEngine,
    app_state: &'a Arc<AppState>,
    app_handle: &'a tauri::AppHandle,
    db_writer: SyncSender<db_worker::DbEvent>,
    recorded_audio: &'a Arc<RwLock<RollingAudioBuffer>>,
    anchor: &'a RecordingAnchor,
    selected_device: Option<&'a str>,
}

pub(crate) struct RecognizeContext {
    segments: Arc<RwLock<Vec<SegmentResult>>>,
    next_realtime_segment_id: Arc<AtomicU64>,
    next_revision: Arc<AtomicU64>,
    correction_engine: CorrectionEngine,
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    db_writer: SyncSender<db_worker::DbEvent>,
    base_wall: chrono::DateTime<Local>,
    audio_offset_samples: u64,
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

fn manifest_assets_dir() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let assets_dir = manifest_dir.join("assets");
    assets_dir.exists().then_some(assets_dir)
}

fn should_prefer_manifest_assets(exe: &Path) -> bool {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    exe.starts_with(manifest_dir.join("target"))
}

fn resource_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        debug!("[resource_dir] current_exe: {exe:?}");

        for ancestor in exe.ancestors() {
            if ancestor.extension().is_some_and(|ext| ext == "app") {
                let resources = ancestor.join("Contents").join("Resources");
                debug!("[resource_dir] found .app bundle: {ancestor:?}");
                if resources.exists() {
                    let assets = resources.join("assets");
                    if assets.exists() {
                        debug!("[resource_dir] using assets inside Resources: {assets:?}");
                        return assets;
                    }
                    debug!("[resource_dir] using Resources directly: {resources:?}");
                    return resources;
                }
                break;
            }
        }

        if let Some(exe_dir) = exe.parent() {
            if should_prefer_manifest_assets(&exe) {
                if let Some(manifest_assets_dir) = manifest_assets_dir() {
                    debug!("[resource_dir] preferring manifest assets dir in dev: {manifest_assets_dir:?}");
                    return manifest_assets_dir;
                }
            }

            let assets_dir = exe_dir.join("assets");
            if assets_dir.exists() {
                debug!("[resource_dir] using assets dir: {assets_dir:?}");
                return assets_dir;
            }
            debug!("[resource_dir] using exe dir: {exe_dir:?}");
            if let Some(manifest_assets_dir) = manifest_assets_dir() {
                debug!("[resource_dir] using manifest assets dir: {manifest_assets_dir:?}");
                return manifest_assets_dir;
            }
            return exe_dir.to_path_buf();
        }
    }
    if let Some(manifest_assets_dir) = manifest_assets_dir() {
        debug!("[resource_dir] using manifest assets dir without current_exe: {manifest_assets_dir:?}");
        return manifest_assets_dir;
    }
    warn!("[resource_dir] fallback to current directory");
    PathBuf::from(".")
}

fn build_models(settings: &VadSettings) -> Result<(OfflineRecognizer, VoiceActivityDetector, u32), String> {
    let dir = resource_dir();
    let model_dir = dir.join(MODEL_NAME);
    let silero_vad_path = dir.join("silero_vad.onnx");

    debug!("[build_models] MODEL_TYPE={MODEL_TYPE}, MODEL_NAME={MODEL_NAME}");
    debug!("[build_models] resource_dir: {dir:?}");
    debug!("[build_models] model_dir: {model_dir:?}, exists={}", model_dir.exists());
    if model_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&model_dir) {
            for entry in entries.flatten() {
                debug!("[build_models]   model_dir entry: {:?}", entry.path());
            }
        }
    } else {
        error!("[build_models] model_dir does not exist");
        debug!("[build_models] dir contents:");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                debug!("[build_models]   {:?}", entry.path());
            }
        } else {
            warn!("[build_models] cannot read dir");
        }
    }
    info!(
        "[build_models] silero_vad: {silero_vad_path:?}, exists={}",
        silero_vad_path.exists()
    );

    let mut asr_config = get_model_config(MODEL_TYPE, &model_dir).ok_or_else(|| {
        format!(
            "Unknown MODEL_TYPE: {MODEL_TYPE}. model_dir={model_dir:?}, exists={}",
            model_dir.exists()
        )
    })?;

    info!(
        "[build_models] got ASR config, num_threads={}",
        asr_config.model_config.num_threads
    );

    let hr_lexicon = dir.join("lexicon.txt");
    if hr_lexicon.exists() {
        debug!("[build_models] using homophone replacer lexicon: {hr_lexicon:?}");
        asr_config.hr.lexicon = hr_lexicon.to_str().map(|s| s.to_string());
    }
    let hr_rule_fst = dir.join("replace.fst");
    if hr_rule_fst.exists() {
        debug!("[build_models] using homophone replacer rule_fst: {hr_rule_fst:?}");
        asr_config.hr.rule_fsts = hr_rule_fst.to_str().map(|s| s.to_string());
    }

    asr_config.model_config.num_threads = settings.num_threads;
    let num_threads = settings.num_threads as u32;

    let silero_vad_str = silero_vad_path
        .to_str()
        .ok_or_else(|| format!("Invalid silero_vad path: {silero_vad_path:?}"))?
        .to_string();

    let vad_config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: Some(silero_vad_str),
            threshold: settings.threshold,
            min_silence_duration: settings.min_silence_duration,
            min_speech_duration: settings.min_speech_duration,
            window_size: 512,
            max_speech_duration: settings.max_speech_duration,
        },
        sample_rate: 16000,
        num_threads: 1,
        ..Default::default()
    };

    info!("[build_models] creating recognizer...");
    let recognizer = OfflineRecognizer::create(&asr_config).ok_or_else(|| {
        format!(
            "Failed to create recognizer. MODEL_TYPE={MODEL_TYPE}, model_dir={model_dir:?}, \
             dir contents: {:?}",
            std::fs::read_dir(&dir)
                .map(|entries| entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>())
                .unwrap_or_default()
        )
    })?;
    info!("[build_models] recognizer created");

    info!("[build_models] creating VAD...");
    let vad = VoiceActivityDetector::create(&vad_config, 120.0).ok_or_else(|| {
        format!(
            "Failed to create VAD. silero_vad={silero_vad_path:?}, exists={}",
            silero_vad_path.exists()
        )
    })?;
    info!("[build_models] VAD created");

    Ok((recognizer, vad, num_threads))
}

fn build_app_state(
    db: db::SpeechDatabase,
    db_writer: SyncSender<db_worker::DbEvent>,
    llm_settings: LlmSettings,
    quality_filter_config: QualityFilterConfig,
    next_segment_id: u64,
    next_revision: u64,
) -> AppState {
    AppState {
        recognizer: Arc::new(RwLock::new(None)),
        vad: Arc::new(RwLock::new(None)),
        recording: Arc::new(AtomicBool::new(false)),
        stop_signal: Arc::new(AtomicBool::new(false)),
        segments: Arc::new(RwLock::new(Vec::new())),
        recorded_audio: Arc::new(RwLock::new(RollingAudioBuffer::new())),
        db: Arc::new(Mutex::new(Some(db))),
        db_writer: Arc::new(db_writer),
        correction_engine: Arc::new(CorrectionEngine::new()),
        start_wall_clock: Arc::new(RwLock::new(None)),
        start_instant: Arc::new(RwLock::new(None)),
        init_status: Arc::new(AtomicU8::new(0)),
        init_error: Arc::new(RwLock::new(String::new())),
        num_threads: Arc::new(AtomicU32::new(0)),
        next_realtime_segment_id: Arc::new(AtomicU64::new(next_segment_id)),
        next_revision: Arc::new(AtomicU64::new(next_revision)),
        settings: Arc::new(RwLock::new(VadSettings::default())),
        llm_settings: Arc::new(RwLock::new(llm_settings)),
        quality_filter_config: Arc::new(RwLock::new(quality_filter_config)),
        llm_models_cache: Arc::new(RwLock::new(None)),
        selected_device: Arc::new(RwLock::new(None)),
        app_handle: Arc::new(RwLock::new(None)),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let workspace = custom_utils::args::workspace(&None, "streaming-speech").unwrap();
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
    let next_segment_id = match tauri::async_runtime::block_on(db.get_next_segment_id()) {
        Ok(next_segment_id) => next_segment_id,
        Err(err) => {
            error!("[db] query next segment_id failed: {err}");
            return;
        }
    };
    let next_revision = match tauri::async_runtime::block_on(db.get_next_revision()) {
        Ok(next_revision) => next_revision,
        Err(err) => {
            error!("[db] query next revision failed: {err}");
            return;
        }
    };
    let db_writer = db_worker::start_db_worker(db.clone());
    let state = build_app_state(
        db,
        db_writer,
        llm_settings,
        quality_filter_config,
        next_segment_id,
        next_revision,
    );
    {
        let db_guard = mutex_lock(&state.db);
        if let Some(db) = db_guard.as_ref() {
            let _ = tauri::async_runtime::block_on(commands::correction::reload_correction_rules(
                db,
                &state.correction_engine,
            ));
        }
    }

    let init_recognizer = Arc::clone(&state.recognizer);
    let init_vad = Arc::clone(&state.vad);
    let init_status = Arc::clone(&state.init_status);
    let init_error = Arc::clone(&state.init_error);
    let init_num_threads = Arc::clone(&state.num_threads);
    let init_settings = Arc::clone(&state.settings);

    tauri::async_runtime::spawn(async move {
        info!("[init] starting model initialization...");
        let settings = read_lock(&init_settings).clone();
        let join = tauri::async_runtime::spawn_blocking(move || build_models(&settings));
        match join.await {
            Ok(Ok((rec, vad, threads))) => {
                info!("[init] models ready, num_threads={threads}");
                {
                    let mut r = write_lock(&init_recognizer);
                    *r = Some(rec);
                }
                {
                    let mut v = write_lock(&init_vad);
                    *v = Some(vad);
                }
                init_num_threads.store(threads, Ordering::Relaxed);
                {
                    let mut s = write_lock(&init_settings);
                    s.num_threads = threads as i32;
                }
                init_status.store(1, Ordering::Relaxed);
            }
            Ok(Err(err)) => {
                error!("[init] model initialization failed: {err}");
                {
                    let mut init_err = write_lock(&init_error);
                    *init_err = err;
                }
                init_status.store(2, Ordering::Relaxed);
            }
            Err(err) => {
                error!("[init] join failed: {err}");
                {
                    let mut init_err = write_lock(&init_error);
                    *init_err = "Internal error: init task join failed".to_string();
                }
                init_status.store(2, Ordering::Relaxed);
            }
        }
    });

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
            commands::correction_api::list_correction_rules,
            commands::correction_api::create_correction_rule,
            commands::correction_api::update_correction_rule,
            commands::correction_api::delete_correction_rule,
            commands::correction_api::reload_correction_rules,
            commands::manual_optimize::manual_optimize_translate,
            commands::quality_filter::get_quality_filter_config,
            commands::quality_filter::save_quality_filter_config,
            commands::quality_filter::reset_quality_filter_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
