pub mod repository;
pub mod schema;

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::Local;
use repository::{CorrectionRule, NewRule, NewSegment, OptimizeResultUpsert, SegmentRow, TranslateResultUpsert};
use rusqlite::Connection;

#[cfg(test)]
#[path = "../lock_utils.rs"]
mod lock_utils;

#[cfg(test)]
use self::lock_utils::mutex_lock;
#[cfg(not(test))]
use crate::lock_utils::mutex_lock;

#[derive(Clone)]
pub struct SpeechDatabase {
    conn: Arc<Mutex<Connection>>,
}

impl SpeechDatabase {
    pub fn init(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create database directory: {}", parent.display()))?;
        }

        let conn =
            Connection::open(db_path).with_context(|| format!("failed to open sqlite db: {}", db_path.display()))?;
        schema::run_migrations(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn ensure_global_scope(&self) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::ensure_global_scope(&conn, &now)
    }

    pub fn touch_global_scope_end(&self) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::touch_global_scope_end(&conn, &now)
    }

    pub fn upsert_segment(&self, segment: NewSegment) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::upsert_segment(&conn, &segment, &now)
    }

    pub fn mark_old_revisions_skipped(&self, latest_revision: i64) -> Result<()> {
        let conn = mutex_lock(&self.conn);
        repository::mark_old_revisions_skipped(&conn, latest_revision)
    }

    pub fn update_optimize_status(&self, revision: i64, status: &str) -> Result<()> {
        let conn = mutex_lock(&self.conn);
        repository::update_optimize_status(&conn, revision, status)
    }

    pub fn update_translate_status(&self, revision: i64, status: &str) -> Result<()> {
        let conn = mutex_lock(&self.conn);
        repository::update_translate_status(&conn, revision, status)
    }

    pub fn upsert_optimize_result(&self, result: OptimizeResultUpsert) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::upsert_optimize_result(&conn, &result, &now)
    }

    pub fn upsert_translate_result(&self, result: TranslateResultUpsert) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::upsert_translate_result(&conn, &result, &now)
    }

    pub fn list_segments(&self, page: u32, page_size: u32) -> Result<Vec<SegmentRow>> {
        let conn = mutex_lock(&self.conn);
        repository::list_segments(&conn, page, page_size)
    }

    pub fn get_next_segment_id(&self) -> Result<u64> {
        let conn = mutex_lock(&self.conn);
        repository::get_next_segment_id(&conn)
    }

    pub fn get_next_revision(&self) -> Result<u64> {
        let conn = mutex_lock(&self.conn);
        repository::get_next_revision(&conn)
    }

    pub fn tail_segments(&self, after_id: i64, limit: u32) -> Result<Vec<SegmentRow>> {
        let conn = mutex_lock(&self.conn);
        repository::tail_segments(&conn, after_id, limit)
    }

    pub fn upsert_rule(&self, rule: NewRule) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::upsert_rule(&conn, &rule, &now)
    }

    pub fn delete_rule(&self, rule_id: i64) -> Result<()> {
        let conn = mutex_lock(&self.conn);
        repository::delete_rule(&conn, rule_id)
    }

    pub fn update_rule(&self, rule_id: i64, rule: NewRule) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::update_rule(&conn, rule_id, &rule, &now)
    }

    pub fn list_rules(&self) -> Result<Vec<CorrectionRule>> {
        let conn = mutex_lock(&self.conn);
        repository::list_rules(&conn)
    }

    pub fn bump_rule_version(&self, checksum: &str) -> Result<i64> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::bump_rule_version(&conn, checksum, &now)
    }

    pub fn get_latest_rule_version(&self) -> Result<Option<(i64, String)>> {
        let conn = mutex_lock(&self.conn);
        repository::get_latest_rule_version(&conn)
    }

    pub fn upsert_setting(&self, key: &str, value: &str) -> Result<()> {
        let now = now_str();
        let conn = mutex_lock(&self.conn);
        repository::upsert_setting(&conn, key, value, &now)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = mutex_lock(&self.conn);
        repository::get_setting(&conn, key)
    }
}

fn now_str() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("streaming-speech-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn init_creates_schema() {
        let path = temp_db_path("schema");
        let db = SpeechDatabase::init(&path).unwrap();
        db.ensure_global_scope().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn segment_and_rule_roundtrip() {
        let path = temp_db_path("roundtrip");
        let db = SpeechDatabase::init(&path).unwrap();
        db.ensure_global_scope().unwrap();

        db.upsert_segment(NewSegment {
            segment_id: 1,
            revision: 1,
            start_sec: 0.0,
            end_sec: 1.0,
            wall_start: "2026-01-01 00:00:00".to_string(),
            wall_end: "2026-01-01 00:00:01".to_string(),
            text_raw: "hello".to_string(),
        })
        .unwrap();

        let segments = db.list_segments(0, 10).unwrap();
        assert_eq!(segments.len(), 1);

        db.upsert_rule(NewRule {
            source: "a".to_string(),
            target: "b".to_string(),
            enabled: true,
            priority: 10,
        })
        .unwrap();
        db.upsert_rule(NewRule {
            source: "a".to_string(),
            target: "b".to_string(),
            enabled: false,
            priority: 20,
        })
        .unwrap();

        let rules = db.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert!(!rules[0].enabled);

        let version = db.bump_rule_version("abc").unwrap();
        assert_eq!(version, 1);
        assert!(db.get_latest_rule_version().unwrap().is_some());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reject_zero_page_size() {
        let path = temp_db_path("paging");
        let db = SpeechDatabase::init(&path).unwrap();
        db.ensure_global_scope().unwrap();
        let err = db.list_segments(0, 0).unwrap_err();
        assert!(err.to_string().contains("page_size"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn next_segment_id_keeps_growing() {
        let path = temp_db_path("next-segment-id");
        let db = SpeechDatabase::init(&path).unwrap();
        db.ensure_global_scope().unwrap();

        assert_eq!(db.get_next_segment_id().unwrap(), 1);

        db.upsert_segment(NewSegment {
            segment_id: 1,
            revision: 1,
            start_sec: 0.0,
            end_sec: 0.8,
            wall_start: "2026-01-01 00:00:00".to_string(),
            wall_end: "2026-01-01 00:00:01".to_string(),
            text_raw: "first".to_string(),
        })
        .unwrap();

        db.upsert_segment(NewSegment {
            segment_id: 3,
            revision: 2,
            start_sec: 1.0,
            end_sec: 1.8,
            wall_start: "2026-01-01 00:00:02".to_string(),
            wall_end: "2026-01-01 00:00:03".to_string(),
            text_raw: "third".to_string(),
        })
        .unwrap();

        assert_eq!(db.get_next_segment_id().unwrap(), 4);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn next_revision_keeps_growing() {
        let path = temp_db_path("next-revision");
        let db = SpeechDatabase::init(&path).unwrap();
        db.ensure_global_scope().unwrap();

        assert_eq!(db.get_next_revision().unwrap(), 1);

        db.upsert_segment(NewSegment {
            segment_id: 10,
            revision: 1,
            start_sec: 0.0,
            end_sec: 0.8,
            wall_start: "2026-01-01 00:00:00".to_string(),
            wall_end: "2026-01-01 00:00:01".to_string(),
            text_raw: "first".to_string(),
        })
        .unwrap();

        db.upsert_segment(NewSegment {
            segment_id: 11,
            revision: 4,
            start_sec: 1.0,
            end_sec: 1.8,
            wall_start: "2026-01-01 00:00:02".to_string(),
            wall_end: "2026-01-01 00:00:03".to_string(),
            text_raw: "fourth".to_string(),
        })
        .unwrap();

        assert_eq!(db.get_next_revision().unwrap(), 5);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn latest_only_marks_old_revisions_skipped_and_keeps_latest_pending() {
        let path = temp_db_path("latest-only");
        let db = SpeechDatabase::init(&path).unwrap();
        db.ensure_global_scope().unwrap();

        for revision in 1..=3 {
            db.upsert_segment(NewSegment {
                segment_id: revision as u64,
                revision,
                start_sec: revision as f32,
                end_sec: revision as f32 + 0.5,
                wall_start: "2026-01-01 00:00:00".to_string(),
                wall_end: "2026-01-01 00:00:01".to_string(),
                text_raw: format!("segment-{revision}"),
            })
            .unwrap();
        }

        db.update_optimize_status(1, "running").unwrap();
        db.update_optimize_status(2, "running").unwrap();
        db.mark_old_revisions_skipped(3).unwrap();

        let segments = db.list_segments(0, 10).unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].revision, 1);
        assert_eq!(segments[0].optimize_status, "failed");
        assert_eq!(segments[1].revision, 2);
        assert_eq!(segments[1].optimize_status, "failed");
        assert_eq!(segments[2].revision, 3);
        assert_eq!(segments[2].optimize_status, "pending");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn llm_result_roundtrip_for_two_stages() {
        let path = temp_db_path("llm-result");
        let db = SpeechDatabase::init(&path).unwrap();
        db.ensure_global_scope().unwrap();

        db.upsert_segment(NewSegment {
            segment_id: 1,
            revision: 1,
            start_sec: 0.0,
            end_sec: 0.8,
            wall_start: "2026-01-01 00:00:00".to_string(),
            wall_end: "2026-01-01 00:00:01".to_string(),
            text_raw: "原始文本".to_string(),
        })
        .unwrap();

        db.upsert_optimize_result(OptimizeResultUpsert {
            revision: 1,
            text_optimized: Some("优化文本".to_string()),
            optimize_error: None,
            optimize_started_at: None,
            optimize_finished_at: None,
        })
        .unwrap();
        db.upsert_translate_result(TranslateResultUpsert {
            revision: 1,
            text_english: Some("optimized english".to_string()),
            translate_error: None,
            translate_started_at: None,
            translate_finished_at: None,
        })
        .unwrap();
        db.update_optimize_status(1, "success").unwrap();
        db.update_translate_status(1, "success").unwrap();

        let segments = db.list_segments(0, 10).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text_raw, "原始文本");
        assert_eq!(segments[0].text_optimized.as_deref(), Some("优化文本"));
        assert_eq!(segments[0].text_english.as_deref(), Some("optimized english"));
        assert_eq!(segments[0].optimize_status, "success");
        assert_eq!(segments[0].translate_status, "success");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn auto_copy_mode_setting_roundtrip_and_legacy_bool_value() {
        let path = temp_db_path("auto-copy-mode");
        let db = SpeechDatabase::init(&path).unwrap();

        db.upsert_setting("llm.auto_copy_mode", "optimized_zh").unwrap();
        let mode = db.get_setting("llm.auto_copy_mode").unwrap();
        assert_eq!(mode.as_deref(), Some("optimized_zh"));

        db.upsert_setting("llm.auto_copy", "false").unwrap();
        let legacy = db.get_setting("llm.auto_copy").unwrap();
        assert_eq!(legacy.as_deref(), Some("false"));

        let _ = std::fs::remove_file(path);
    }
}
