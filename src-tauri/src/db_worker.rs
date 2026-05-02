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

pub(crate) fn start_db_worker(db: db::SpeechDatabase) -> SyncSender<DbEvent> {
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
