use std::sync::mpsc::{self, SyncSender};

use log::{debug, error, warn};

use crate::db;
use crate::db::repository::{NewSegment, OptimizeResultUpsert, TranslateResultUpsert};

pub(crate) const DB_EVENT_QUEUE_CAPACITY: usize = 1024;

#[derive(Clone)]
pub(crate) enum DbEvent {
    InsertSegment {
        segment: NewSegment,
    },
    MarkOptimizeRunning {
        revision: i64,
    },
    MarkOptimizeSuccess {
        revision: i64,
    },
    MarkTranslatePending {
        revision: i64,
    },
    MarkTranslateRunning {
        revision: i64,
    },
    MarkTranslateFailed {
        revision: i64,
    },
    MarkSkippedBefore {
        revision: i64,
    },
    MarkSkipped {
        revision: i64,
    },
    MarkOptimizeFailed {
        revision: i64,
    },
    SaveOptimizeResult {
        revision: i64,
        text_optimized: String,
    },
    SaveTranslateResult {
        revision: i64,
        text_english: String,
    },
    UpdateDiscardResult {
        revision: i64,
        is_discarded: bool,
        discard_reason: Option<String>,
        discard_source: Option<String>,
        discard_confidence: Option<f32>,
        quality_check_status: String,
    },
    TouchGlobalScopeEnd,
}

pub(crate) fn start_db_worker(db: db::SpeechDatabase) -> SyncSender<DbEvent> {
    let (tx, rx) = mpsc::sync_channel::<DbEvent>(DB_EVENT_QUEUE_CAPACITY);
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = rx.recv() {
            match event {
                DbEvent::InsertSegment { segment } => {
                    debug!(
                        "[db-worker] upsert segment segment_id={}, revision={}",
                        segment.segment_id, segment.revision
                    );
                    if let Err(err) = db.upsert_segment(segment.clone()).await {
                        error!(
                            "[db-worker] upsert failed segment_id={}, revision={}, err={}",
                            segment.segment_id, segment.revision, err
                        );
                    } else {
                        debug!(
                            "[db-worker] upsert ok segment_id={}, revision={}",
                            segment.segment_id, segment.revision
                        );
                    }
                }
                DbEvent::MarkOptimizeRunning { revision } => {
                    debug!("[db-worker] mark running revision={}", revision);
                    let _ = db.update_optimize_status(revision, "running".to_string()).await;
                }
                DbEvent::MarkSkippedBefore { revision } => {
                    debug!("[db-worker] mark skipped before revision={}", revision);
                    let _ = db.mark_old_revisions_skipped(revision).await;
                }
                DbEvent::MarkSkipped { revision } => {
                    debug!("[db-worker] mark skipped revision={}", revision);
                    let _ = db.update_optimize_status(revision, "failed".to_string()).await;
                    let _ = db.update_translate_status(revision, "blocked".to_string()).await;
                }
                DbEvent::MarkOptimizeFailed { revision } => {
                    warn!("[db-worker] mark failed revision={}", revision);
                    let _ = db.update_optimize_status(revision, "failed".to_string()).await;
                    let _ = db.update_translate_status(revision, "blocked".to_string()).await;
                }
                DbEvent::MarkOptimizeSuccess { revision } => {
                    let _ = db.update_optimize_status(revision, "success".to_string()).await;
                }
                DbEvent::MarkTranslatePending { revision } => {
                    let _ = db.update_translate_status(revision, "pending".to_string()).await;
                }
                DbEvent::MarkTranslateRunning { revision } => {
                    let _ = db.update_translate_status(revision, "running".to_string()).await;
                }
                DbEvent::MarkTranslateFailed { revision } => {
                    let _ = db.update_translate_status(revision, "failed".to_string()).await;
                }
                DbEvent::SaveOptimizeResult {
                    revision,
                    text_optimized,
                } => {
                    let _ = db
                        .upsert_optimize_result(OptimizeResultUpsert {
                            revision,
                            text_optimized: Some(text_optimized),
                            optimize_error: None,
                            optimize_started_at: None,
                            optimize_finished_at: None,
                        })
                        .await;
                }
                DbEvent::SaveTranslateResult { revision, text_english } => {
                    let _ = db
                        .upsert_translate_result(TranslateResultUpsert {
                            revision,
                            text_english: Some(text_english),
                            translate_error: None,
                            translate_started_at: None,
                            translate_finished_at: None,
                        })
                        .await;
                    let _ = db.update_translate_status(revision, "success".to_string()).await;
                }
                DbEvent::TouchGlobalScopeEnd => {
                    let _ = db.touch_global_scope_end().await;
                }
                DbEvent::UpdateDiscardResult {
                    revision,
                    is_discarded,
                    discard_reason,
                    discard_source,
                    discard_confidence,
                    quality_check_status,
                } => {
                    let _ = db
                        .update_discard_result(
                            revision,
                            is_discarded,
                            discard_reason,
                            discard_source,
                            discard_confidence,
                            quality_check_status,
                        )
                        .await;
                }
            }
        }
    });
    tx
}
