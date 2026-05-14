use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use log::{error, info, warn};
use tauri::Emitter;
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::commands::history::to_segment_dto;
use crate::db::repository::{OptimizeResultUpsert, TranslateResultUpsert};
use crate::db::SpeechDatabase;
use crate::llm_client::{optimize_text, translate_text};
use crate::llm_settings::{AutoCopyMode, LlmSettings};
use crate::{mutex_lock, read_lock, update_segment_llm_state, AppState, SegmentResult};

const STATUS_PENDING: &str = "pending";
const STATUS_RUNNING: &str = "running";
const STATUS_SUCCESS: &str = "success";
const STATUS_FAILED: &str = "failed";
const STATUS_BLOCKED: &str = "blocked";
const MANUAL_BUSY_ERROR: &str = "该分段正在处理中，请稍后再试";

#[tauri::command]
pub async fn manual_optimize_translate(
    revision: i64,
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    info!("[manual_optimize_translate] triggered for revision={revision}");

    if revision <= 0 {
        return Err("revision must be greater than 0".to_string());
    }

    let db = clone_database(&state)?;
    let text_raw = validate_manual_target(&state, &db, revision).await?;
    let settings = read_lock(&state.llm_settings).clone();
    reset_manual_state(&db, &state.segments, &app_handle, revision)
        .await
        .map_err(|err| err.to_string())?;

    let segments = Arc::clone(&state.segments);
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_manual_optimize_translate(revision, text_raw, db, settings, segments, app_handle).await {
            error!("[manual_optimize_translate] task failed for revision={revision}: {err:#}");
        }
    });

    Ok(())
}

async fn validate_manual_target(
    state: &tauri::State<'_, AppState>,
    db: &SpeechDatabase,
    revision: i64,
) -> Result<String, String> {
    {
        let segments = read_lock(&state.segments);
        if let Some(segment) = segments.iter().find(|segment| segment.revision == revision) {
            if is_segment_processing(&segment.optimize_status, &segment.translate_status) {
                return Err(MANUAL_BUSY_ERROR.to_string());
            }

            let text_raw = segment.text.trim();
            if text_raw.is_empty() {
                return Err("text raw is empty".to_string());
            }
            return Ok(text_raw.to_string());
        }
    }

    let Some(segment) = db
        .get_segment_by_revision(revision)
        .await
        .map_err(|err| format!("query revision {revision} failed: {err}"))?
    else {
        return Err(format!(
            "未找到该历史分段（revision={revision}），可能已被清理或 revision 无效"
        ));
    };

    if is_segment_processing(&segment.optimize_status, &segment.translate_status) {
        return Err(MANUAL_BUSY_ERROR.to_string());
    }

    let text_raw = segment.text_raw.trim();
    if text_raw.is_empty() {
        return Err("text raw is empty".to_string());
    }
    Ok(text_raw.to_string())
}

async fn reset_manual_state(
    db: &SpeechDatabase,
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: &tauri::AppHandle,
    revision: i64,
) -> Result<()> {
    db.update_optimize_status(revision, STATUS_PENDING.to_string())
        .await
        .with_context(|| format!("failed to reset optimize status for revision={revision}"))?;
    db.update_translate_status(revision, STATUS_BLOCKED.to_string())
        .await
        .with_context(|| format!("failed to reset translate status for revision={revision}"))?;

    update_segment_llm_state(
        segments,
        revision,
        Some(STATUS_PENDING),
        Some(STATUS_BLOCKED),
        Some(String::new()),
        Some(String::new()),
    );
    emit_segment_updated(db, app_handle, revision).await;
    Ok(())
}

async fn run_manual_optimize_translate(
    revision: i64,
    text_raw: String,
    db: SpeechDatabase,
    settings: LlmSettings,
    segments: Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: tauri::AppHandle,
) -> Result<()> {
    mark_optimize_running(&db, &segments, &app_handle, revision).await?;

    let optimized = match optimize_text(&settings, &text_raw).await {
        Ok(optimized) => optimized,
        Err(err) => {
            mark_optimize_failed(&db, &segments, &app_handle, revision).await?;
            return Err(anyhow::anyhow!("optimize failed for revision={revision}: {err}"));
        }
    };

    save_optimize_result(&db, &segments, &app_handle, revision, optimized.clone()).await?;
    maybe_copy_optimized_result(&app_handle, &settings.auto_copy_mode, &optimized, revision);
    mark_translate_running(&db, &segments, &app_handle, revision).await?;

    let english = match translate_text(&settings, &optimized).await {
        Ok(english) => english,
        Err(err) => {
            mark_translate_failed(&db, &segments, &app_handle, revision).await?;
            return Err(anyhow::anyhow!("translate failed for revision={revision}: {err}"));
        }
    };

    save_translate_result(&db, &segments, &app_handle, revision, english.clone()).await?;
    maybe_copy_translated_result(&app_handle, &settings.auto_copy_mode, &english, revision);
    info!("[manual_optimize_translate] finished for revision={revision}");
    Ok(())
}

async fn mark_optimize_running(
    db: &SpeechDatabase,
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: &tauri::AppHandle,
    revision: i64,
) -> Result<()> {
    db.update_optimize_status(revision, STATUS_RUNNING.to_string())
        .await
        .with_context(|| format!("failed to mark optimize running for revision={revision}"))?;
    update_segment_llm_state(segments, revision, Some(STATUS_RUNNING), None, None, None);
    emit_segment_updated(db, app_handle, revision).await;
    Ok(())
}

async fn save_optimize_result(
    db: &SpeechDatabase,
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: &tauri::AppHandle,
    revision: i64,
    optimized: String,
) -> Result<()> {
    db.upsert_optimize_result(OptimizeResultUpsert {
        revision,
        text_optimized: Some(optimized.clone()),
        optimize_error: None,
        optimize_started_at: None,
        optimize_finished_at: None,
    })
    .await
    .with_context(|| format!("failed to save optimize result for revision={revision}"))?;
    db.update_optimize_status(revision, STATUS_SUCCESS.to_string())
        .await
        .with_context(|| format!("failed to mark optimize success for revision={revision}"))?;
    db.update_translate_status(revision, STATUS_PENDING.to_string())
        .await
        .with_context(|| format!("failed to mark translate pending for revision={revision}"))?;

    update_segment_llm_state(
        segments,
        revision,
        Some(STATUS_SUCCESS),
        Some(STATUS_PENDING),
        Some(optimized),
        None,
    );
    emit_segment_updated(db, app_handle, revision).await;
    Ok(())
}

async fn mark_translate_running(
    db: &SpeechDatabase,
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: &tauri::AppHandle,
    revision: i64,
) -> Result<()> {
    db.update_translate_status(revision, STATUS_RUNNING.to_string())
        .await
        .with_context(|| format!("failed to mark translate running for revision={revision}"))?;
    update_segment_llm_state(segments, revision, None, Some(STATUS_RUNNING), None, None);
    emit_segment_updated(db, app_handle, revision).await;
    Ok(())
}

async fn save_translate_result(
    db: &SpeechDatabase,
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: &tauri::AppHandle,
    revision: i64,
    english: String,
) -> Result<()> {
    db.upsert_translate_result(TranslateResultUpsert {
        revision,
        text_english: Some(english.clone()),
        translate_error: None,
        translate_started_at: None,
        translate_finished_at: None,
    })
    .await
    .with_context(|| format!("failed to save translate result for revision={revision}"))?;
    db.update_translate_status(revision, STATUS_SUCCESS.to_string())
        .await
        .with_context(|| format!("failed to mark translate success for revision={revision}"))?;

    update_segment_llm_state(segments, revision, None, Some(STATUS_SUCCESS), None, Some(english));
    emit_segment_updated(db, app_handle, revision).await;
    Ok(())
}

async fn mark_optimize_failed(
    db: &SpeechDatabase,
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: &tauri::AppHandle,
    revision: i64,
) -> Result<()> {
    db.update_optimize_status(revision, STATUS_FAILED.to_string())
        .await
        .with_context(|| format!("failed to mark optimize failed for revision={revision}"))?;
    db.update_translate_status(revision, STATUS_BLOCKED.to_string())
        .await
        .with_context(|| format!("failed to mark translate blocked for revision={revision}"))?;

    update_segment_llm_state(
        segments,
        revision,
        Some(STATUS_FAILED),
        Some(STATUS_BLOCKED),
        None,
        None,
    );
    emit_segment_updated(db, app_handle, revision).await;
    Ok(())
}

async fn mark_translate_failed(
    db: &SpeechDatabase,
    segments: &Arc<RwLock<Vec<SegmentResult>>>,
    app_handle: &tauri::AppHandle,
    revision: i64,
) -> Result<()> {
    db.update_translate_status(revision, STATUS_FAILED.to_string())
        .await
        .with_context(|| format!("failed to mark translate failed for revision={revision}"))?;
    update_segment_llm_state(segments, revision, None, Some(STATUS_FAILED), None, None);
    emit_segment_updated(db, app_handle, revision).await;
    Ok(())
}

async fn emit_segment_updated(db: &SpeechDatabase, app_handle: &tauri::AppHandle, revision: i64) {
    match db.get_segment_by_revision(revision).await {
        Ok(Some(row)) => {
            if let Err(err) = app_handle.emit("segment_updated", to_segment_dto(row)) {
                warn!("[manual_optimize_translate] emit segment_updated failed for revision={revision}: {err}");
            }
        }
        Ok(None) => {
            warn!("[manual_optimize_translate] emit skipped, revision not found: {revision}");
        }
        Err(err) => {
            warn!("[manual_optimize_translate] query revision for emit failed, revision={revision}: {err}");
        }
    }
}

fn maybe_copy_optimized_result(
    app_handle: &tauri::AppHandle,
    auto_copy_mode: &AutoCopyMode,
    optimized: &str,
    revision: i64,
) {
    if !matches!(auto_copy_mode, &AutoCopyMode::OptimizedZh) {
        return;
    }

    if let Err(err) = app_handle.clipboard().write_text(optimized) {
        warn!("[manual_optimize_translate] copy 优化中文 failed for revision={revision}: {err}");
    }
}

fn maybe_copy_translated_result(
    app_handle: &tauri::AppHandle,
    auto_copy_mode: &AutoCopyMode,
    english: &str,
    revision: i64,
) {
    if !matches!(auto_copy_mode, &AutoCopyMode::English) {
        return;
    }

    if let Err(err) = app_handle.clipboard().write_text(english) {
        warn!("[manual_optimize_translate] copy 英文 failed for revision={revision}: {err}");
    }
}

fn clone_database(state: &tauri::State<'_, AppState>) -> Result<SpeechDatabase, String> {
    let guard = mutex_lock(&state.db);
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "database not initialized".to_string())
}

fn is_segment_processing(optimize_status: &str, translate_status: &str) -> bool {
    matches!(optimize_status, STATUS_PENDING | STATUS_RUNNING)
        || matches!(translate_status, STATUS_PENDING | STATUS_RUNNING)
}
