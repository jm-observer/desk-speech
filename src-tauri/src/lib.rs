mod audio_buffer;
mod commands;
mod correction;
pub mod db;
mod llm_client;
mod llm_settings;
mod model_registry;
mod settings;

use audio_buffer::{RollingAudioBuffer, SAMPLE_RATE};
use model_registry::get_model_config;
use serde::{Deserialize, Serialize};
use sherpa_onnx::{LinearResampler, OfflineRecognizer, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::Context;
use tokio::sync::{Mutex, RwLock};

use crate::correction::CorrectionEngine;
use crate::db::repository::{NewSegment, OptimizeResultUpsert, TranslateResultUpsert};
use crate::llm_client::{
    list_models as llm_list_models, model_cache_valid, optimize_text, translate_text, CachedModels,
};
use crate::llm_settings::{validate_llm_settings, AutoCopyMode, LlmSettings};
use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use log::{debug, error, info, warn};
use rusqlite::Connection;
use tauri_plugin_clipboard_manager::ClipboardExt;
use crate::db::schema;

const DB_EVENT_QUEUE_CAPACITY: usize = 1024;
const MERGE_MAX_DURATION_SEC: f32 = 30.0;
const MERGE_MAX_GAP_SEC: f32 = 5.6;

#[derive(Clone)]
enum DbEvent {
    InsertSegment {
        segment: NewSegment,
    },
    MarkOptimizeRunning {
        session_id: String,
        revision: i64,
    },
    MarkOptimizeSuccess {
        session_id: String,
        revision: i64,
    },
    MarkTranslatePending {
        session_id: String,
        revision: i64,
    },
    MarkTranslateRunning {
        session_id: String,
        revision: i64,
    },
    MarkTranslateFailed {
        session_id: String,
        revision: i64,
    },
    MarkSkippedBefore {
        session_id: String,
        revision: i64,
    },
    MarkSkipped {
        session_id: String,
        revision: i64,
    },
    MarkOptimizeFailed {
        session_id: String,
        revision: i64,
    },
    SaveOptimizeResult {
        session_id: String,
        revision: i64,
        text_optimized: String,
    },
    SaveTranslateResult {
        session_id: String,
        revision: i64,
        text_english: String,
    },
    CloseSession {
        session_id: String,
    },
}

fn merge_segment_in_place(segments: &mut [SegmentResult], incoming: &SegmentResult) -> bool {
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

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct VadSettings {
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    max_speech_duration: f32,
    num_threads: i32,
}

impl Default for VadSettings {
    fn default() -> Self {
        Self {
            threshold: 0.2,
            min_silence_duration: 0.2,
            min_speech_duration: 0.2,
            max_speech_duration: 10.0,
            num_threads: 2,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case")]
enum SegmentUpdateType {
    Append,
    Replace,
}

#[derive(Serialize, Clone)]
struct SegmentResult {
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

fn update_segment_llm_state(
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
    db_writer: Arc<Mutex<Option<SyncSender<DbEvent>>>>,
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

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CombinedSettings {
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    max_speech_duration: f32,
    num_threads: i32,
    provider_url: String,
    api_key: String,
    selected_model: String,
    optimize_prompt_template: String,
    translate_prompt_template: String,
    auto_copy_mode: AutoCopyMode,
}

#[derive(Serialize)]
pub(crate) struct ModelListResponse {
    models: Vec<String>,
}

struct RecordingAnchor {
    base_wall: chrono::DateTime<Local>,
    audio_offset: u64,
}

struct RecordingRuntime<'a> {
    stop_signal: &'a AtomicBool,
    segments: &'a Arc<RwLock<Vec<SegmentResult>>>,
    next_realtime_segment_id: &'a AtomicU64,
    next_revision: &'a AtomicU64,
    correction_engine: &'a CorrectionEngine,
    app_state: &'a Arc<AppState>,
    app_handle: &'a tauri::AppHandle,
    db_writer: Option<&'a SyncSender<DbEvent>>,
    session_id: &'a str,
    recorded_audio: &'a Arc<RwLock<RollingAudioBuffer>>,
    anchor: &'a RecordingAnchor,
    selected_device: Option<&'a str>,
}

struct RecognizeContext<'a> {
    segments: &'a Arc<RwLock<Vec<SegmentResult>>>,
    next_realtime_segment_id: &'a AtomicU64,
    next_revision: &'a AtomicU64,
    correction_engine: &'a CorrectionEngine,
    state: &'a Arc<AppState>,
    app_handle: &'a tauri::AppHandle,
    session_id: &'a str,
    db_writer: Option<&'a SyncSender<DbEvent>>,
    base_wall: &'a chrono::DateTime<Local>,
    audio_offset_samples: u64,
}

// ---------------------------------------------------------------------------
// cpal microphone capture
// ---------------------------------------------------------------------------

fn build_input_stream(device: &cpal::Device, tx: mpsc::Sender<Vec<f32>>) -> Result<cpal::Stream, String> {
    let supported = device
        .default_input_config()
        .map_err(|e| format!("No input config: {e}"))?;
    let config = supported.config();
    let sample_format = supported.sample_format();
    let channels = config.channels as usize;
    if channels == 0 {
        return Err("Device reports 0 channels".to_string());
    }

    info!(
        "[mic] format: {:?}, channels: {}, sample_rate: {}",
        sample_format, channels, config.sample_rate.0
    );

    let err_fn = |err| info!("[mic] stream error: {:?}", err);

    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                &config,
                move |data: &[f32], _| {
                    if data.is_empty() {
                        return;
                    }
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|frame| {
                            let sum: f32 = frame.iter().copied().sum();
                            sum / channels as f32
                        })
                        .collect();
                    let _ = tx.send(mono);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Build F32 stream: {e}"))?,

        SampleFormat::I16 => device
            .build_input_stream(
                &config,
                move |data: &[i16], _| {
                    if data.is_empty() {
                        return;
                    }
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|frame| {
                            let sum: f32 = frame.iter().map(|&s| s as f32 / i16::MAX as f32).sum();
                            sum / channels as f32
                        })
                        .collect();
                    let _ = tx.send(mono);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Build I16 stream: {e}"))?,

        SampleFormat::U16 => device
            .build_input_stream(
                &config,
                move |data: &[u16], _| {
                    if data.is_empty() {
                        return;
                    }
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|frame| {
                            let sum: f32 = frame.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum();
                            sum / channels as f32
                        })
                        .collect();
                    let _ = tx.send(mono);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Build U16 stream: {e}"))?,

        other => return Err(format!("Unsupported sample format: {:?}", other)),
    };

    Ok(stream)
}

// ---------------------------------------------------------------------------
// Recognize a single VAD speech segment
// ---------------------------------------------------------------------------

fn recognize_segment(recognizer: &OfflineRecognizer, segment: &sherpa_onnx::SpeechSegment, ctx: &RecognizeContext<'_>) {
    let samples = segment.samples();
    let duration = samples.len() as f32 / SAMPLE_RATE as f32;
    if duration < 0.1 {
        return;
    }

    let vad_start = segment.start() as f32 / SAMPLE_RATE as f32;
    let offset_secs = ctx.audio_offset_samples as f32 / SAMPLE_RATE as f32;
    let rel_start = offset_secs + vad_start;
    let rel_end = rel_start + duration;

    let stream = recognizer.create_stream();
    stream.accept_waveform(16000, samples);
    recognizer.decode(&stream);

    if let Some(r) = stream.get_result() {
        let text_raw = r.text.trim().to_string();
        if !text_raw.is_empty()
            && !text_raw
                .chars()
                .all(|c| c.is_ascii_punctuation() || c.is_ascii_whitespace())
        {
            let text_corrected = ctx.correction_engine.apply(&text_raw);
            let revision = ctx.next_revision.fetch_add(1, Ordering::Relaxed) as i64;
            let wall_start = *ctx.base_wall + chrono::Duration::milliseconds((vad_start * 1000.0) as i64);
            let wall_end = *ctx.base_wall + chrono::Duration::milliseconds(((vad_start + duration) * 1000.0) as i64);

            let wall_start_fmt = wall_start.format("%Y-%m-%d %H:%M:%S").to_string();
            let wall_end_fmt = wall_end.format("%Y-%m-%d %H:%M:%S").to_string();
            let new_segment = SegmentResult {
                segment_id: ctx.next_realtime_segment_id.fetch_add(1, Ordering::Relaxed),
                revision,
                update_type: SegmentUpdateType::Append,
                start: rel_start,
                end: rel_end,
                wall_start: wall_start_fmt.clone(),
                wall_end: wall_end_fmt.clone(),
                text: text_corrected.clone(),
                text_optimized: None,
                text_english: None,
                optimize_status: "pending".to_string(),
                translate_status: "blocked".to_string(),
            };

            let db_segment_id = {
                let mut segs = ctx.segments.blocking_write();
                if merge_segment_in_place(&mut segs, &new_segment) {
                    segs.last().map(|s| s.segment_id).unwrap_or(new_segment.segment_id)
                } else {
                    let id = new_segment.segment_id;
                    segs.push(new_segment);
                    id
                }
            };

            let llm_input_text = ctx
                .segments
                .blocking_read()
                .last()
                .map(|seg| seg.text.clone())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| text_corrected.clone());

            if let Some(writer) = ctx.db_writer {
                let event = DbEvent::InsertSegment {
                    segment: NewSegment {
                        session_id: ctx.session_id.to_string(),
                        segment_id: db_segment_id,
                        revision,
                        start_sec: rel_start,
                        end_sec: rel_end,
                        wall_start: wall_start_fmt,
                        wall_end: wall_end_fmt,
                        text_raw,
                    },
                };
                if let Err(err) = writer.try_send(event) {
                    if matches!(err, TrySendError::Full(_)) {
                        warn!("[db-worker] queue full, dropping segment event");
                    } else {
                        warn!(
                            "[db-worker] failed to enqueue segment session_id={}, segment_id={}, revision={}, err={}",
                            ctx.session_id, db_segment_id, revision, err
                        );
                    }
                } else {
                    debug!(
                        "[db-worker] enqueued segment session_id={}, segment_id={}, revision={}",
                        ctx.session_id, db_segment_id, revision
                    );
                }
            } else {
                warn!("[db-worker] writer not ready, fallback to direct upsert");
                {
                    let db_guard = ctx.state.db.blocking_lock();
                    if let Some(db) = db_guard.as_ref() {
                        let result = db.upsert_segment(NewSegment {
                            session_id: ctx.session_id.to_string(),
                            segment_id: db_segment_id,
                            revision,
                            start_sec: rel_start,
                            end_sec: rel_end,
                            wall_start: wall_start_fmt.clone(),
                            wall_end: wall_end_fmt.clone(),
                            text_raw: text_raw.clone(),
                        });
                        match result {
                            Ok(()) => info!(
                                "[db-direct] upsert ok session_id={}, segment_id={}, revision={}",
                                ctx.session_id, db_segment_id, revision
                            ),
                            Err(err) => error!(
                                "[db-direct] upsert failed session_id={}, segment_id={}, revision={}, err={}",
                                ctx.session_id, db_segment_id, revision, err
                            ),
                        }
                    }
                }
            }

            if let Some(writer) = ctx.db_writer {
                spawn_llm_postprocess_task_v2(
                    writer.clone(),
                    Arc::clone(ctx.state),
                    ctx.app_handle.clone(),
                    ctx.session_id.to_string(),
                    revision,
                    llm_input_text,
                );
            }
        }
    }
}

fn spawn_llm_postprocess_task_v2(
    writer: SyncSender<DbEvent>,
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
    session_id: String,
    revision: i64,
    llm_input_text: String,
) {
    tauri::async_runtime::spawn(async move {
        info!(
            "[llm] start postprocess session_id={}, revision={}, text_len={}",
            session_id,
            revision,
            llm_input_text.len()
        );
        update_segment_llm_state(&state.segments, revision, Some("running"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkSkippedBefore {
            session_id: session_id.clone(),
            revision,
        });
        let _ = writer.try_send(DbEvent::MarkOptimizeRunning {
            session_id: session_id.clone(),
            revision,
        });

        let settings = state.llm_settings.blocking_read().clone();

        if settings.selected_model.trim().is_empty() {
            match llm_list_models(&settings).await {
                Ok(models) => {
                    if let Some(first) = models.into_iter().find(|m| !m.trim().is_empty()) {
                        warn!(
                            "[llm] selected_model is empty, fallback to first model={}, session_id={}, revision={}",
                            first, session_id, revision
                        );
                        let mut fallback_settings = settings.clone();
                        fallback_settings.selected_model = first;
                        perform_postprocess_and_copy(
                            &writer,
                            &state,
                            &app_handle,
                            &session_id,
                            revision,
                            &llm_input_text,
                            fallback_settings,
                        )
                        .await;
                        return;
                    } else {
                        warn!(
                            "[llm] skip due to empty model list, session_id={}, revision={}",
                            session_id, revision
                        );
                        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
                        let _ = writer.try_send(DbEvent::MarkOptimizeFailed { session_id, revision });
                        return;
                    }
                }
                Err(err) => {
                    warn!(
                        "[llm] skip due to empty model and list_models failed, session_id={}, revision={}, err={}",
                        session_id, revision, err
                    );
                    update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
                    let _ = writer.try_send(DbEvent::MarkOptimizeFailed { session_id, revision });
                    return;
                }
            }
        }

        perform_postprocess_and_copy(
            &writer,
            &state,
            &app_handle,
            &session_id,
            revision,
            &llm_input_text,
            settings,
        )
        .await;
    });
}

async fn perform_postprocess_and_copy(
    writer: &SyncSender<DbEvent>,
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    session_id: &str,
    revision: i64,
    llm_input_text: &str,
    settings: LlmSettings,
) {
    let optimized = match optimize_text(&settings, llm_input_text).await {
        Ok(v) => v,
        Err(err) => {
            error!("llm postprocess failed: {}", err);
            update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
            let _ = writer.try_send(DbEvent::MarkOptimizeFailed {
                session_id: session_id.to_string(),
                revision,
            });
            return;
        }
    };

    let latest_revision = state.next_revision.load(Ordering::Relaxed) as i64 - 1;
    if revision < latest_revision {
        info!(
            "[llm] revision skipped as stale, session_id={}, revision={}, latest_revision={}",
            session_id, revision, latest_revision
        );
        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkSkipped {
            session_id: session_id.to_string(),
            revision,
        });
        return;
    }

    let optimized_for_memory = optimized.clone();
    let result = DbEvent::SaveOptimizeResult {
        session_id: session_id.to_string(),
        revision,
        text_optimized: optimized,
    };
    if writer.try_send(result).is_err() {
        update_segment_llm_state(&state.segments, revision, Some("failed"), None, None, None);
        let _ = writer.try_send(DbEvent::MarkOptimizeFailed {
            session_id: session_id.to_string(),
            revision,
        });
        return;
    }
    info!(
        "[llm] optimize done, session_id={}, revision={}, optimized_len={}",
        session_id,
        revision,
        optimized_for_memory.len()
    );
    let _ = writer.try_send(DbEvent::MarkOptimizeSuccess {
        session_id: session_id.to_string(),
        revision,
    });
    let _ = writer.try_send(DbEvent::MarkTranslatePending {
        session_id: session_id.to_string(),
        revision,
    });
    update_segment_llm_state(
        &state.segments,
        revision,
        Some("success"),
        Some("pending"),
        Some(optimized_for_memory.clone()),
        None,
    );

    let _ = writer.try_send(DbEvent::MarkTranslateRunning {
        session_id: session_id.to_string(),
        revision,
    });
    update_segment_llm_state(&state.segments, revision, None, Some("running"), None, None);

    let english = match translate_text(&settings, &optimized_for_memory).await {
        Ok(v) => v,
        Err(err) => {
            error!("llm translate failed: {}", err);
            let _ = writer.try_send(DbEvent::MarkTranslateFailed {
                session_id: session_id.to_string(),
                revision,
            });
            update_segment_llm_state(&state.segments, revision, None, Some("failed"), None, None);
            return;
        }
    };

    let result = DbEvent::SaveTranslateResult {
        session_id: session_id.to_string(),
        revision,
        text_english: english.clone(),
    };
    if writer.try_send(result).is_err() {
        let _ = writer.try_send(DbEvent::MarkTranslateFailed {
            session_id: session_id.to_string(),
            revision,
        });
        update_segment_llm_state(&state.segments, revision, None, Some("failed"), None, None);
        return;
    }

    info!(
        "[llm] translate done, session_id={}, revision={}, english_len={}",
        session_id,
        revision,
        english.len()
    );
    update_segment_llm_state(
        &state.segments,
        revision,
        None,
        Some("success"),
        None,
        Some(english.clone()),
    );

    let copy_text = match settings.auto_copy_mode {
        AutoCopyMode::Off => None,
        AutoCopyMode::English => Some((english, "英文")),
        AutoCopyMode::OptimizedZh => Some((optimized_for_memory, "优化中文")),
    };

    if let Some((text, mode_name)) = copy_text {
        if let Err(err) = app_handle.clipboard().write_text(text) {
            error!("copy {mode_name} to clipboard failed: {}", err);
        } else {
            info!(
                "[llm] auto copy done, session_id={}, revision={}, mode={}",
                session_id, revision, mode_name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Device enumeration
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub(crate) struct InputDevice {
    name: String,
    is_default: bool,
}

pub(crate) fn list_input_devices() -> Result<Vec<InputDevice>, String> {
    info!("[list_input_devices]");
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let devices: Vec<InputDevice> = host
        .input_devices()
        .map_err(|e| format!("Cannot enumerate devices: {e}"))?
        .filter_map(|d| {
            let name = d.name().ok()?;
            Some(InputDevice {
                is_default: name == default_name,
                name,
            })
        })
        .collect();

    Ok(devices)
}

pub(crate) fn set_input_device(device_name: Option<String>, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[set_input_device] device_name={:?}", device_name);
    if state.recording.load(Ordering::SeqCst) {
        return Err("Cannot change device while recording".to_string());
    }
    *state.selected_device.blocking_write() = device_name;
    Ok(())
}

pub(crate) fn get_selected_device(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    info!("[get_selected_device]");
    Ok(state.selected_device.blocking_read().clone())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

pub(crate) fn start_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[start_recording]");
    if state.recording.swap(true, Ordering::SeqCst) {
        return Err("Already recording".to_string());
    }

    let init = state.init_status.load(Ordering::Relaxed);
    if init != 1 {
        state.recording.store(false, Ordering::SeqCst);
        return Err("Models not ready".to_string());
    }

    // Take recognizer and VAD out of shared state for exclusive use
    let recognizer = {
        let mut guard = state.recognizer.blocking_write();
        guard.take().ok_or("Recognizer not available")?
    };
    let vad = {
        let mut guard = state.vad.blocking_write();
        guard.take().ok_or("VAD not available")?
    };

    state.stop_signal.store(false, Ordering::Relaxed);

    // Get current audio length as offset for new segments
    let audio_offset = state.recorded_audio.blocking_read().global_end_sample();

    let now = Local::now();
    *state.start_wall_clock.blocking_write() = Some(now);
    *state.start_instant.blocking_write() = Some(Instant::now());

    info!("[start_recording] starting at {now}");

    let session_id = {
        let db_guard = state.db.blocking_lock();
        let db = db_guard.as_ref().ok_or("Database not initialized")?;
        db.create_session().map_err(|e| e.to_string())?
    };
    *state.current_session_id.blocking_write() = Some(session_id.clone());
    let stop_signal = Arc::clone(&state.stop_signal);
    let recording = Arc::clone(&state.recording);
    let segments = Arc::clone(&state.segments);
    let recorded_audio = Arc::clone(&state.recorded_audio);
    let recognizer_arc = Arc::clone(&state.recognizer);
    let vad_arc = Arc::clone(&state.vad);
    let correction_engine = Arc::clone(&state.correction_engine);
    let app_state = Arc::new(AppState {
        recognizer: Arc::clone(&state.recognizer),
        vad: Arc::clone(&state.vad),
        recording: Arc::clone(&state.recording),
        stop_signal: Arc::clone(&state.stop_signal),
        segments: Arc::clone(&state.segments),
        recorded_audio: Arc::clone(&state.recorded_audio),
        db: Arc::clone(&state.db),
        db_writer: Arc::clone(&state.db_writer),
        current_session_id: Arc::clone(&state.current_session_id),
        correction_engine: Arc::clone(&state.correction_engine),
        start_wall_clock: Arc::clone(&state.start_wall_clock),
        start_instant: Arc::clone(&state.start_instant),
        init_status: Arc::clone(&state.init_status),
        init_error: Arc::clone(&state.init_error),
        num_threads: Arc::clone(&state.num_threads),
        next_realtime_segment_id: Arc::clone(&state.next_realtime_segment_id),
        next_revision: Arc::clone(&state.next_revision),
        settings: Arc::clone(&state.settings),
        llm_settings: Arc::clone(&state.llm_settings),
        llm_models_cache: Arc::clone(&state.llm_models_cache),
        selected_device: Arc::clone(&state.selected_device),
    });
    let db_writer = state.db_writer.blocking_lock().as_ref().cloned();
    info!(
        "[start_recording] created session_id={}, db_writer_ready={}",
        session_id,
        db_writer.is_some()
    );
    let current_session_id = Arc::clone(&state.current_session_id);
    let next_realtime_segment_id = Arc::clone(&state.next_realtime_segment_id);
    let next_revision = Arc::clone(&state.next_revision);
    let anchor = RecordingAnchor {
        base_wall: now,
        audio_offset,
    };
    let selected_device = state.selected_device.blocking_write().clone();

    let db_writer_for_run = db_writer.clone();
    let session_id_for_run = session_id.clone();
    tauri::async_runtime::spawn(async move {
        let join = tauri::async_runtime::spawn_blocking(move || {
            run_recording(
                recognizer,
                vad,
                RecordingRuntime {
                    stop_signal: &stop_signal,
                    segments: &segments,
                    next_realtime_segment_id: &next_realtime_segment_id,
                    next_revision: &next_revision,
                    correction_engine: &correction_engine,
                    app_state: &app_state,
                    app_handle: &app,
                    db_writer: db_writer_for_run.as_ref(),
                    session_id: &session_id_for_run,
                    recorded_audio: &recorded_audio,
                    anchor: &anchor,
                    selected_device: selected_device.as_deref(),
                },
            )
        });

        match join.await {
            Ok(Ok((rec, v))) => {
                {
                    let mut r = recognizer_arc.blocking_write();
                    *r = Some(rec);
                }
                {
                    let mut va = vad_arc.blocking_write();
                    *va = Some(v);
                }
            }
            Ok(Err(err)) => {
                error!("[recording task] error: {err}");
            }
            Err(err) => {
                error!("[recording task] join failed: {err}");
            }
        }

        recording.store(false, Ordering::SeqCst);
        if let Some(writer) = db_writer.as_ref() {
            let _ = writer.try_send(DbEvent::CloseSession {
                session_id: session_id.clone(),
            });
        }
        {
            let mut guard = current_session_id.blocking_write();
            *guard = None;
        }
        info!("[recording task] stopped");
    });

    Ok(())
}

fn run_recording(
    recognizer: OfflineRecognizer,
    vad: VoiceActivityDetector,
    runtime: RecordingRuntime<'_>,
) -> Result<(OfflineRecognizer, VoiceActivityDetector), String> {
    let host = cpal::default_host();
    let device = if let Some(name) = runtime.selected_device {
        host.input_devices()
            .map_err(|e| format!("Cannot enumerate devices: {e}"))?
            .find(|d| d.name().ok().as_deref() == Some(name))
            .ok_or_else(|| format!("Device not found: {name}"))?
    } else {
        host.default_input_device().ok_or("No default input device")?
    };

    info!("[recording] device: {:?}", device.name().unwrap_or_default());

    let supported = device
        .default_input_config()
        .map_err(|e| format!("No input config: {e}"))?;
    let mic_sample_rate = supported.sample_rate().0 as i32;

    let resampler = if mic_sample_rate != SAMPLE_RATE as i32 {
        Some(
            LinearResampler::create(mic_sample_rate, SAMPLE_RATE as i32)
                .ok_or_else(|| format!("Failed to create resampler for {mic_sample_rate} Hz"))?,
        )
    } else {
        None
    };

    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let stream = build_input_stream(&device, tx)?;
    stream.play().map_err(|e| format!("Stream play: {e}"))?;

    vad.reset();
    let window_size: usize = 512;
    let mut vad_buf: Vec<f32> = Vec::new();

    loop {
        if runtime.stop_signal.load(Ordering::Relaxed) {
            break;
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(samples) => {
                let pcm = if let Some(ref resamp) = resampler {
                    resamp.resample(&samples, false)
                } else {
                    samples
                };

                {
                    let mut audio = runtime.recorded_audio.blocking_write();
                    audio.push_samples(&pcm);
                }

                vad_buf.extend_from_slice(&pcm);

                while vad_buf.len() >= window_size {
                    vad.accept_waveform(&vad_buf[..window_size]);
                    vad_buf.drain(..window_size);

                    while let Some(segment) = vad.front() {
                        let recognize_ctx = RecognizeContext {
                            segments: runtime.segments,
                            next_realtime_segment_id: runtime.next_realtime_segment_id,
                            next_revision: runtime.next_revision,
                            correction_engine: runtime.correction_engine,
                            state: runtime.app_state,
                            app_handle: runtime.app_handle,
                            session_id: runtime.session_id,
                            db_writer: runtime.db_writer,
                            base_wall: &runtime.anchor.base_wall,
                            audio_offset_samples: runtime.anchor.audio_offset,
                        };
                        recognize_segment(&recognizer, &segment, &recognize_ctx);
                        vad.pop();
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("[recording] audio channel disconnected");
                break;
            }
        }
    }

    // Drop the stream to stop capture before flushing
    drop(stream);

    // Feed any remaining samples (zero-pad to window size)
    if !vad_buf.is_empty() {
        vad_buf.resize(window_size, 0.0);
        vad.accept_waveform(&vad_buf[..window_size]);
    }

    // Flush VAD unconditionally — it may have buffered speech internally
    vad.flush();
    while let Some(segment) = vad.front() {
        let recognize_ctx = RecognizeContext {
            segments: runtime.segments,
            next_realtime_segment_id: runtime.next_realtime_segment_id,
            next_revision: runtime.next_revision,
            correction_engine: runtime.correction_engine,
            state: runtime.app_state,
            app_handle: runtime.app_handle,
            session_id: runtime.session_id,
            db_writer: runtime.db_writer,
            base_wall: &runtime.anchor.base_wall,
            audio_offset_samples: runtime.anchor.audio_offset,
        };
        recognize_segment(&recognizer, &segment, &recognize_ctx);
        vad.pop();
    }

    let seg_count = runtime.segments.blocking_read().len();
    info!("[recording] flushed, total segments: {seg_count}");

    Ok((recognizer, vad))
}

pub(crate) fn stop_recording(state: tauri::State<'_, AppState>) {
    info!("[stop_recording]");
    info!("[stop_recording] signalling stop");
    state.stop_signal.store(true, Ordering::Relaxed);
}

pub(crate) fn clear_results(state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[clear_results]");
    if state.recording.load(Ordering::SeqCst) {
        return Err("Cannot clear while recording".to_string());
    }
    state.segments.blocking_write().clear();
    state.recorded_audio.blocking_write().clear();
    *state.start_wall_clock.blocking_write() = None;
    *state.start_instant.blocking_write() = None;
    info!("[clear_results] cleared all segments and audio");
    Ok(())
}

pub(crate) fn get_recording_state(state: tauri::State<'_, AppState>) -> Result<RecordingState, String> {
    info!("[get_recording_state]");
    let recording = state.recording.load(Ordering::Relaxed);
    let segments = state.segments.blocking_write().clone();
    let elapsed_secs = state
        .start_instant
        .blocking_read()
        .map(|i| i.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    let (audio_window_start_sec, audio_window_end_sec) = {
        let audio = state.recorded_audio.blocking_write();
        (
            audio.global_start_sample() as f32 / SAMPLE_RATE as f32,
            audio.global_end_sample() as f32 / SAMPLE_RATE as f32,
        )
    };
    Ok(RecordingState {
        recording,
        segments,
        elapsed_secs,
        audio_window_start_sec,
        audio_window_end_sec,
    })
}

pub(crate) fn list_sessions(
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<commands::history::DbSessionDto>, String> {
    info!("[list_sessions] page={}, page_size={}", page, page_size);
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::history::list_sessions(db, page, page_size)
}

pub(crate) fn list_session_segments(
    session_id: String,
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<commands::history::DbSegmentDto>, String> {
    info!(
        "[list_session_segments] session_id={}, page={}, page_size={}",
        session_id, page, page_size
    );
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::history::list_session_segments(db, &session_id, page, page_size)
}

pub(crate) fn tail_session_segments(
    session_id: String,
    after_id: i64,
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<commands::history::DbSegmentDto>, String> {
    info!(
        "[tail_session_segments] session_id={}, after_id={}, limit={}",
        session_id, after_id, limit
    );
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::history::tail_session_segments(db, &session_id, after_id, limit)
}

pub(crate) fn list_correction_rules(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<commands::correction::CorrectionRuleDto>, String> {
    info!("[list_correction_rules]");
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::correction::list_correction_rules(db)
}

pub(crate) fn create_correction_rule(
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!(
        "[create_correction_rule] source={}, target={}, priority={}, enabled={}",
        source, target, priority, enabled
    );
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::correction::create_correction_rule(db, &state.correction_engine, source, target, priority, enabled)
}

pub(crate) fn update_correction_rule(
    id: i64,
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!(
        "[update_correction_rule] id={}, source={}, target={}, priority={}, enabled={}",
        id, source, target, priority, enabled
    );
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::correction::update_correction_rule(db, &state.correction_engine, id, source, target, priority, enabled)
}

pub(crate) fn delete_correction_rule(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[delete_correction_rule] id={}", id);
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::correction::delete_correction_rule(db, &state.correction_engine, id)
}

pub(crate) fn reload_correction_rules(state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[reload_correction_rules]");
    let db = state.db.blocking_lock();
    let db = db.as_ref().ok_or("Database not initialized")?;
    commands::correction::reload_correction_rules(db, &state.correction_engine)
}

pub(crate) fn save_segment_as_wav(
    path: String,
    start: f32,
    end: f32,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!("[save_segment_as_wav] path={}, start={}, end={}", path, start, end);
    let audio = state.recorded_audio.blocking_write();
    if audio.len() == 0 {
        return Err("No recorded audio".to_string());
    }

    let start_sample = (start * SAMPLE_RATE as f32) as u64;
    let end_sample = (end * SAMPLE_RATE as f32) as u64;
    if start_sample >= end_sample {
        return Err("Invalid time range".to_string());
    }

    let segment = audio
        .snapshot_range(start_sample, end_sample)
        .ok_or("Requested segment is outside in-memory window")?;
    write_wav(&path, &segment)
}

pub(crate) fn save_all_audio(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[save_all_audio] path={}", path);
    let audio = state.recorded_audio.blocking_write();
    if audio.len() == 0 {
        return Err("No recorded audio".to_string());
    }
    let samples = audio.snapshot_all();
    write_wav(&path, &samples)
}

pub(crate) fn get_recorded_audio_path(state: tauri::State<'_, AppState>) -> Result<String, String> {
    info!("[get_recorded_audio_path]");
    let audio = state.recorded_audio.blocking_write();
    if audio.len() == 0 {
        return Err("No recorded audio".to_string());
    }

    let tmp = std::env::temp_dir().join(format!("sherpa-onnx-mic-{}.wav", std::process::id()));
    let tmp_str = tmp.to_str().ok_or("Invalid temp path")?.to_string();
    let samples = audio.snapshot_all();
    write_wav(&tmp_str, &samples)?;
    info!("[get_recorded_audio_path] wrote {tmp_str} ({} samples)", samples.len());
    Ok(tmp_str)
}

fn format_srt_time(seconds: f32) -> String {
    let total_ms = (seconds * 1000.0) as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

pub(crate) fn export_srt(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[export_srt] path={}", path);
    let segments = state.segments.blocking_write();
    if segments.is_empty() {
        return Err("No results to export".to_string());
    }

    let mut srt = String::new();
    for (i, seg) in segments.iter().enumerate() {
        srt.push_str(&format!("{}\n", i + 1));
        srt.push_str(&format!(
            "{} --> {}\n",
            format_srt_time(seg.start),
            format_srt_time(seg.end)
        ));
        srt.push_str(&seg.text);
        srt.push_str("\n\n");
    }

    std::fs::write(&path, srt).map_err(|e| format!("Cannot write file: {e}"))?;
    Ok(())
}

pub(crate) fn copy_text_to_clipboard(app: tauri::AppHandle, text: String) -> Result<(), String> {
    info!("[copy_text_to_clipboard] text_len={}", text.len());
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// Write mono f32 PCM samples as a 16-bit WAV file at 16 kHz.
fn write_wav(path: &str, samples: &[f32]) -> Result<(), String> {
    let num_samples = samples.len() as u32;
    let byte_rate = 16000u32 * 2;
    let data_size = num_samples * 2;
    let file_size = 36 + data_size;

    let f = File::create(path).map_err(|e| format!("Cannot create file: {e}"))?;
    let mut w = std::io::BufWriter::new(f);

    use std::io::Write;
    w.write_all(b"RIFF").map_err(|e| e.to_string())?;
    w.write_all(&file_size.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(b"WAVE").map_err(|e| e.to_string())?;
    w.write_all(b"fmt ").map_err(|e| e.to_string())?;
    w.write_all(&16u32.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&16000u32.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&byte_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&2u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&16u16.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(b"data").map_err(|e| e.to_string())?;
    w.write_all(&data_size.to_le_bytes()).map_err(|e| e.to_string())?;
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let pcm = (clamped * 32767.0) as i16;
        w.write_all(&pcm.to_le_bytes()).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Init status
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub(crate) struct InitStatus {
    status: u8,
    error: String,
    num_threads: u32,
}

pub(crate) fn get_init_status(state: tauri::State<'_, AppState>) -> InitStatus {
    info!("[get_init_status]");
    let status = state.init_status.load(Ordering::Relaxed);
    let error = state.init_error.blocking_read().clone();
    let num_threads = state.num_threads.load(Ordering::Relaxed);
    InitStatus {
        status,
        error,
        num_threads,
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub(crate) fn get_settings(state: tauri::State<'_, AppState>) -> Result<CombinedSettings, String> {
    info!("[get_settings]");
    let vad = state.settings.blocking_write().clone();
    let llm = state.llm_settings.blocking_write().clone();
    Ok(CombinedSettings {
        threshold: vad.threshold,
        min_silence_duration: vad.min_silence_duration,
        min_speech_duration: vad.min_speech_duration,
        max_speech_duration: vad.max_speech_duration,
        num_threads: vad.num_threads,
        provider_url: llm.provider_url,
        api_key: llm.api_key,
        selected_model: llm.selected_model,
        optimize_prompt_template: llm.optimize_prompt_template,
        translate_prompt_template: llm.translate_prompt_template,
        auto_copy_mode: llm.auto_copy_mode,
    })
}

fn validate_settings(s: &VadSettings) -> Result<(), String> {
    if s.threshold <= 0.0 || s.threshold >= 1.0 {
        return Err("threshold must be between 0.0 and 1.0 (exclusive)".to_string());
    }
    if s.min_silence_duration < 0.0 {
        return Err("min_silence_duration must be >= 0".to_string());
    }
    if s.min_speech_duration < 0.0 {
        return Err("min_speech_duration must be >= 0".to_string());
    }
    if s.max_speech_duration <= 0.0 {
        return Err("max_speech_duration must be > 0".to_string());
    }
    if s.num_threads < 1 || s.num_threads > 16 {
        return Err("num_threads must be between 1 and 16".to_string());
    }
    Ok(())
}

pub(crate) fn apply_settings(new_settings: CombinedSettings, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[apply_settings]");
    if state.recording.load(Ordering::SeqCst) {
        return Err("Cannot change settings while recording".to_string());
    }
    let init = state.init_status.load(Ordering::Relaxed);
    if init == 0 {
        return Err("Models are still loading, please wait".to_string());
    }

    let new_vad_settings = VadSettings {
        threshold: new_settings.threshold,
        min_silence_duration: new_settings.min_silence_duration,
        min_speech_duration: new_settings.min_speech_duration,
        max_speech_duration: new_settings.max_speech_duration,
        num_threads: new_settings.num_threads,
    };
    let new_llm_settings = LlmSettings {
        provider_url: new_settings.provider_url,
        api_key: new_settings.api_key,
        selected_model: new_settings.selected_model,
        optimize_prompt_template: new_settings.optimize_prompt_template,
        translate_prompt_template: new_settings.translate_prompt_template,
        auto_copy_mode: new_settings.auto_copy_mode,
    };

    validate_settings(&new_vad_settings)?;
    validate_llm_settings(&new_llm_settings)?;

    {
        let current_vad = state.settings.blocking_write();
        let current_llm = state.llm_settings.blocking_write();
        if *current_vad == new_vad_settings && *current_llm == new_llm_settings {
            return Ok(());
        }
    }

    state.init_status.store(0, Ordering::Relaxed);
    *state.settings.blocking_write() = new_vad_settings.clone();
    *state.llm_settings.blocking_write() = new_llm_settings.clone();
    *state.llm_models_cache.blocking_write() = None;

    {
        let db = state.db.blocking_lock();
        let db = db.as_ref().ok_or("Database not initialized")?;
        db.upsert_setting("llm.provider_url", &new_llm_settings.provider_url)
            .map_err(|e| e.to_string())?;
        db.upsert_setting("llm.api_key", &new_llm_settings.api_key)
            .map_err(|e| e.to_string())?;
        db.upsert_setting("llm.selected_model", &new_llm_settings.selected_model)
            .map_err(|e| e.to_string())?;
        db.upsert_setting(
            "llm.optimize_prompt_template",
            &new_llm_settings.optimize_prompt_template,
        )
        .map_err(|e| e.to_string())?;
        db.upsert_setting(
            "llm.translate_prompt_template",
            &new_llm_settings.translate_prompt_template,
        )
        .map_err(|e| e.to_string())?;
        db.upsert_setting(
            "llm.auto_copy_mode",
            match new_llm_settings.auto_copy_mode {
                AutoCopyMode::Off => "off",
                AutoCopyMode::English => "english",
                AutoCopyMode::OptimizedZh => "optimized_zh",
            },
        )
        .map_err(|e| e.to_string())?;
    }

    let recognizer_arc = Arc::clone(&state.recognizer);
    let vad_arc = Arc::clone(&state.vad);
    let init_status = Arc::clone(&state.init_status);
    let init_error = Arc::clone(&state.init_error);
    let init_num_threads = Arc::clone(&state.num_threads);

    tauri::async_runtime::spawn(async move {
        info!("[apply_settings] rebuilding models...");
        let join = tauri::async_runtime::spawn_blocking(move || build_models(&new_vad_settings));
        match join.await {
            Ok(Ok((rec, vad, threads))) => {
                info!("[apply_settings] models rebuilt, num_threads={threads}");
                {
                    let mut r = recognizer_arc.blocking_write();
                    *r = Some(rec);
                }
                {
                    let mut v = vad_arc.blocking_write();
                    *v = Some(vad);
                }
                init_num_threads.store(threads, Ordering::Relaxed);
                init_status.store(1, Ordering::Relaxed);
            }
            Ok(Err(err)) => {
                error!("[apply_settings] rebuild failed: {err}");
                {
                    let mut init_err = init_error.blocking_write();
                    *init_err = err;
                }
                init_status.store(2, Ordering::Relaxed);
            }
            Err(err) => {
                error!("[apply_settings] join failed: {err}");
                {
                    let mut init_err = init_error.blocking_write();
                    *init_err = "Internal error: settings task join failed".to_string();
                }
                init_status.store(2, Ordering::Relaxed);
            }
        }
    });

    Ok(())
}

pub(crate) async fn list_llm_models(state: tauri::State<'_, AppState>) -> Result<ModelListResponse, String> {
    info!("[list_llm_models]");
    let settings = state.llm_settings.blocking_write().clone();
    validate_llm_settings(&settings)?;

    if let Some(cache) = state.llm_models_cache.blocking_write().as_ref() {
        if model_cache_valid(cache) {
            return Ok(ModelListResponse {
                models: cache.models.clone(),
            });
        }
    }

    let fetched = llm_list_models(&settings).await?;
    *state.llm_models_cache.blocking_write() = Some(CachedModels {
        fetched_at: Instant::now(),
        models: fetched.clone(),
    });
    Ok(ModelListResponse { models: fetched })
}

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

fn load_llm_settings_from_db(db: &db::SpeechDatabase) -> LlmSettings {
    let mut settings = LlmSettings::default();

    if let Ok(Some(v)) = db.get_setting("llm.provider_url") {
        settings.provider_url = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.api_key") {
        settings.api_key = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.selected_model") {
        settings.selected_model = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.optimize_prompt_template") {
        settings.optimize_prompt_template = v;
    } else if let Ok(Some(v)) = db.get_setting("llm.prompt_template") {
        settings.optimize_prompt_template = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.translate_prompt_template") {
        settings.translate_prompt_template = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.auto_copy_mode") {
        settings.auto_copy_mode = match v.as_str() {
            "off" => AutoCopyMode::Off,
            "optimized_zh" => AutoCopyMode::OptimizedZh,
            _ => AutoCopyMode::English,
        };
    } else if let Ok(Some(v)) = db.get_setting("llm.auto_copy") {
        settings.auto_copy_mode = if v == "false" || v == "0" {
            AutoCopyMode::Off
        } else {
            AutoCopyMode::English
        };
    }

    settings
}

fn start_db_worker(db: db::SpeechDatabase) -> SyncSender<DbEvent> {
    let (tx, rx) = mpsc::sync_channel::<DbEvent>(DB_EVENT_QUEUE_CAPACITY);
    tauri::async_runtime::spawn(async move {
        let join = tauri::async_runtime::spawn_blocking(move || {
            while let Ok(event) = rx.recv() {
                match event {
                    DbEvent::InsertSegment { segment } => {
                        debug!(
                            "[db-worker] upsert segment session_id={}, segment_id={}, revision={}",
                            segment.session_id, segment.segment_id, segment.revision
                        );
                        if let Err(err) = db.upsert_segment(segment.clone()) {
                            error!(
                                "[db-worker] upsert failed session_id={}, segment_id={}, revision={}, err={}",
                                segment.session_id, segment.segment_id, segment.revision, err
                            );
                        } else {
                            debug!(
                                "[db-worker] upsert ok session_id={}, segment_id={}, revision={}",
                                segment.session_id, segment.segment_id, segment.revision
                            );
                        }
                    }
                    DbEvent::MarkOptimizeRunning { session_id, revision } => {
                        debug!(
                            "[db-worker] mark running session_id={}, revision={}",
                            session_id, revision
                        );
                        let _ = db.update_optimize_status(&session_id, revision, "running");
                    }
                    DbEvent::MarkSkippedBefore { session_id, revision } => {
                        debug!(
                            "[db-worker] mark skipped before session_id={}, revision={}",
                            session_id, revision
                        );
                        let _ = db.mark_old_revisions_skipped(&session_id, revision);
                    }
                    DbEvent::MarkSkipped { session_id, revision } => {
                        debug!(
                            "[db-worker] mark skipped session_id={}, revision={}",
                            session_id, revision
                        );
                        let _ = db.update_optimize_status(&session_id, revision, "failed");
                        let _ = db.update_translate_status(&session_id, revision, "blocked");
                    }
                    DbEvent::MarkOptimizeFailed { session_id, revision } => {
                        warn!(
                            "[db-worker] mark failed session_id={}, revision={}",
                            session_id, revision
                        );
                        let _ = db.update_optimize_status(&session_id, revision, "failed");
                        let _ = db.update_translate_status(&session_id, revision, "blocked");
                    }
                    DbEvent::MarkOptimizeSuccess { session_id, revision } => {
                        let _ = db.update_optimize_status(&session_id, revision, "success");
                    }
                    DbEvent::MarkTranslatePending { session_id, revision } => {
                        let _ = db.update_translate_status(&session_id, revision, "pending");
                    }
                    DbEvent::MarkTranslateRunning { session_id, revision } => {
                        let _ = db.update_translate_status(&session_id, revision, "running");
                    }
                    DbEvent::MarkTranslateFailed { session_id, revision } => {
                        let _ = db.update_translate_status(&session_id, revision, "failed");
                    }
                    DbEvent::SaveOptimizeResult {
                        session_id,
                        revision,
                        text_optimized,
                    } => {
                        let _ = db.upsert_optimize_result(OptimizeResultUpsert {
                            session_id,
                            revision,
                            text_optimized: Some(text_optimized),
                            optimize_error: None,
                            optimize_started_at: None,
                            optimize_finished_at: None,
                        });
                    }
                    DbEvent::SaveTranslateResult {
                        session_id,
                        revision,
                        text_english,
                    } => {
                        let _ = db.upsert_translate_result(TranslateResultUpsert {
                            session_id: session_id.clone(),
                            revision,
                            text_english: Some(text_english),
                            translate_error: None,
                            translate_started_at: None,
                            translate_finished_at: None,
                        });
                        let _ = db.update_translate_status(&session_id, revision, "success");
                    }
                    DbEvent::CloseSession { session_id } => {
                        let _ = db.close_session(&session_id);
                    }
                }
            }
        });
        if let Err(err) = join.await {
            error!("[db-worker] join failed: {err}");
        }
    });
    tx
}

fn build_app_state(db: db::SpeechDatabase, db_writer: SyncSender<DbEvent>, llm_settings: LlmSettings) -> AppState {
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

    let llm_settings = load_llm_settings_from_db(&db);
    let db_writer = start_db_worker(db.clone());
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
        let settings = init_settings.blocking_read().clone();
        let join = tauri::async_runtime::spawn_blocking(move || build_models(&settings));
        match join.await {
            Ok(Ok((rec, vad, threads))) => {
                info!("[init] models ready, num_threads={threads}");
                {
                    let mut r = init_recognizer.blocking_write();
                    *r = Some(rec);
                }
                {
                    let mut v = init_vad.blocking_write();
                    *v = Some(vad);
                }
                init_num_threads.store(threads, Ordering::Relaxed);
                {
                    let mut s = init_settings.blocking_write();
                    s.num_threads = threads as i32;
                }
                init_status.store(1, Ordering::Relaxed);
            }
            Ok(Err(err)) => {
                error!("[init] model initialization failed: {err}");
                {
                    let mut init_err = init_error.blocking_write();
                    *init_err = err;
                }
                init_status.store(2, Ordering::Relaxed);
            }
            Err(err) => {
                error!("[init] join failed: {err}");
                {
                    let mut init_err = init_error.blocking_write();
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
