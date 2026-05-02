mod audio_buffer;
mod commands;
mod correction;
pub mod db;
mod db_worker;
mod llm_client;
mod llm_settings;
mod model_registry;
mod settings;

use audio_buffer::RollingAudioBuffer;
use model_registry::get_model_config;
use serde::Serialize;
use sherpa_onnx::{OfflineRecognizer, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

use crate::correction::CorrectionEngine;
use crate::llm_client::CachedModels;
use crate::llm_settings::LlmSettings;
use crate::settings::VadSettings;
use chrono::Local;
use log::{debug, error, info, warn};

const MERGE_MAX_DURATION_SEC: f32 = 30.0;
const MERGE_MAX_GAP_SEC: f32 = 5.6;

pub(crate) fn merge_segment_in_place(segments: &mut [SegmentResult], incoming: &SegmentResult) -> bool {
    let Some(last) = segments.last_mut() else {
        return false;
    };

    let gap_sec = incoming.start - last.end;
    let merged_duration = incoming.end - last.start;
    if !(0.0..=MERGE_MAX_GAP_SEC).contains(&gap_sec) || merged_duration > MERGE_MAX_DURATION_SEC {
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
    last.update_type = SegmentUpdateType::Replace;
    true
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
}

pub(crate) fn update_segment_llm_state(
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    revision: i64,
    optimize_status: Option<&str>,
    translate_status: Option<&str>,
    optimized: Option<String>,
    english: Option<String>,
) {
    {
        let mut segs = segments.blocking_write();
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
    db_writer: Arc<Mutex<Option<SyncSender<db_worker::DbEvent>>>>,
    current_session_id: Arc<RwLock<Option<String>>>,
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
    llm_models_cache: Arc<RwLock<Option<CachedModels>>>,
    selected_device: Arc<RwLock<Option<String>>>,
}

pub(crate) struct RecordingAnchor {
    base_wall: chrono::DateTime<Local>,
    audio_offset: u64,
}

pub(crate) struct RecordingRuntime<'a> {
    stop_signal: &'a AtomicBool,
    segments: &'a Arc<RwLock<Vec<SegmentResult>>>,
    next_realtime_segment_id: &'a AtomicU64,
    next_revision: &'a AtomicU64,
    correction_engine: &'a CorrectionEngine,
    app_state: &'a Arc<AppState>,
    app_handle: &'a tauri::AppHandle,
    db_writer: Option<&'a SyncSender<db_worker::DbEvent>>,
    session_id: &'a str,
    recorded_audio: &'a Arc<RwLock<RollingAudioBuffer>>,
    anchor: &'a RecordingAnchor,
    selected_device: Option<&'a str>,
}

pub(crate) struct RecognizeContext<'a> {
    segments: &'a Arc<RwLock<Vec<SegmentResult>>>,
    next_realtime_segment_id: &'a AtomicU64,
    next_revision: &'a AtomicU64,
    correction_engine: &'a CorrectionEngine,
    state: &'a Arc<AppState>,
    app_handle: &'a tauri::AppHandle,
    session_id: &'a str,
    db_writer: Option<&'a SyncSender<db_worker::DbEvent>>,
    base_wall: &'a chrono::DateTime<Local>,
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
            let assets_dir = exe_dir.join("assets");
            if assets_dir.exists() {
                debug!("[resource_dir] using assets dir: {assets_dir:?}");
                return assets_dir;
            }
            debug!("[resource_dir] using exe dir: {exe_dir:?}");
            return exe_dir.to_path_buf();
        }
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
) -> AppState {
    AppState {
        recognizer: Arc::new(RwLock::new(None)),
        vad: Arc::new(RwLock::new(None)),
        recording: Arc::new(AtomicBool::new(false)),
        stop_signal: Arc::new(AtomicBool::new(false)),
        segments: Arc::new(RwLock::new(Vec::new())),
        recorded_audio: Arc::new(RwLock::new(RollingAudioBuffer::new())),
        db: Arc::new(Mutex::new(Some(db))),
        db_writer: Arc::new(Mutex::new(Some(db_writer))),
        current_session_id: Arc::new(RwLock::new(None)),
        correction_engine: Arc::new(CorrectionEngine::new()),
        start_wall_clock: Arc::new(RwLock::new(None)),
        start_instant: Arc::new(RwLock::new(None)),
        init_status: Arc::new(AtomicU8::new(0)),
        init_error: Arc::new(RwLock::new(String::new())),
        num_threads: Arc::new(AtomicU32::new(0)),
        next_realtime_segment_id: Arc::new(AtomicU64::new(1)),
        next_revision: Arc::new(AtomicU64::new(1)),
        settings: Arc::new(RwLock::new(VadSettings::default())),
        llm_settings: Arc::new(RwLock::new(llm_settings)),
        llm_models_cache: Arc::new(RwLock::new(None)),
        selected_device: Arc::new(RwLock::new(None)),
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
    let db = match db::SpeechDatabase::init(&db_path) {
        Ok(db) => db,
        Err(err) => {
            error!("[db] init failed at {}: {err}", db_path.display());
            return;
        }
    };

    let llm_settings = settings::load_llm_settings_from_db(&db);
    let db_writer = db_worker::start_db_worker(db.clone());
    let state = build_app_state(db, db_writer, llm_settings);
    {
        let db_guard = state.db.blocking_lock();
        if let Some(db) = db_guard.as_ref() {
            let _ = commands::correction::reload_correction_rules(db, &state.correction_engine);
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
        let settings = init_settings.read().await.clone();
        let join = tauri::async_runtime::spawn_blocking(move || build_models(&settings));
        match join.await {
            Ok(Ok((rec, vad, threads))) => {
                info!("[init] models ready, num_threads={threads}");
                {
                    let mut r = init_recognizer.write().await;
                    *r = Some(rec);
                }
                {
                    let mut v = init_vad.write().await;
                    *v = Some(vad);
                }
                init_num_threads.store(threads, Ordering::Relaxed);
                {
                    let mut s = init_settings.write().await;
                    s.num_threads = threads as i32;
                }
                init_status.store(1, Ordering::Relaxed);
            }
            Ok(Err(err)) => {
                error!("[init] model initialization failed: {err}");
                {
                    let mut init_err = init_error.write().await;
                    *init_err = err;
                }
                init_status.store(2, Ordering::Relaxed);
            }
            Err(err) => {
                error!("[init] join failed: {err}");
                {
                    let mut init_err = init_error.write().await;
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
        .setup({
            let correction_engine = Arc::clone(&state.correction_engine);
            let db_state = Arc::clone(&state.db);
            move |app| {
                let _ = app;
                let db_guard = db_state.blocking_lock();
                if let Some(db) = db_guard.as_ref() {
                    let _ = commands::correction::reload_correction_rules(db, &correction_engine);
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
            commands::history_api::list_sessions,
            commands::history_api::list_session_segments,
            commands::history_api::tail_session_segments,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
