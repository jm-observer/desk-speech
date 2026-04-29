pub mod repository;
pub mod schema;

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::Local;
use repository::{CorrectionRule, NewRule, NewSegment, SegmentRow, SessionRow};
use rusqlite::Connection;
use uuid::Uuid;

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

    pub fn create_session(&self) -> Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let now = now_str();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::create_session(&conn, &session_id, &now)
    }

    pub fn close_session(&self, session_id: &str) -> Result<()> {
        let now = now_str();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::close_session(&conn, session_id, &now)
    }

    pub fn insert_segment(&self, segment: NewSegment) -> Result<()> {
        let now = now_str();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::insert_segment(&conn, &segment, &now)
    }

    pub fn list_segments(&self, session_id: &str, page: u32, page_size: u32) -> Result<Vec<SegmentRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::list_segments(&conn, session_id, page, page_size)
    }

    pub fn list_sessions(&self, page: u32, page_size: u32) -> Result<Vec<SessionRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::list_sessions(&conn, page, page_size)
    }

    pub fn tail_segments(&self, session_id: &str, after_id: i64, limit: u32) -> Result<Vec<SegmentRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::tail_segments(&conn, session_id, after_id, limit)
    }

    pub fn session_exists(&self, session_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::session_exists(&conn, session_id)
    }

    pub fn upsert_rule(&self, rule: NewRule) -> Result<()> {
        let now = now_str();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::upsert_rule(&conn, &rule, &now)
    }

    pub fn delete_rule(&self, rule_id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::delete_rule(&conn, rule_id)
    }

    pub fn update_rule(&self, rule_id: i64, rule: NewRule) -> Result<()> {
        let now = now_str();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::update_rule(&conn, rule_id, &rule, &now)
    }

    pub fn list_rules(&self) -> Result<Vec<CorrectionRule>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::list_rules(&conn)
    }

    pub fn bump_rule_version(&self, checksum: &str) -> Result<i64> {
        let now = now_str();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::bump_rule_version(&conn, checksum, &now)
    }

    pub fn get_latest_rule_version(&self) -> Result<Option<(i64, String)>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        repository::get_latest_rule_version(&conn)
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
        let session_id = db.create_session().unwrap();
        assert!(!session_id.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn segment_and_rule_roundtrip() {
        let path = temp_db_path("roundtrip");
        let db = SpeechDatabase::init(&path).unwrap();
        let session_id = db.create_session().unwrap();

        db.insert_segment(NewSegment {
            session_id: session_id.clone(),
            start_sec: 0.0,
            end_sec: 1.0,
            wall_start: "2026-01-01 00:00:00".to_string(),
            wall_end: "2026-01-01 00:00:01".to_string(),
            text_raw: "hello".to_string(),
            text_corrected: "hello".to_string(),
        })
        .unwrap();

        let segments = db.list_segments(&session_id, 0, 10).unwrap();
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
        let session_id = db.create_session().unwrap();
        let err = db.list_segments(&session_id, 0, 0).unwrap_err();
        assert!(err.to_string().contains("page_size"));
        let _ = std::fs::remove_file(path);
    }
}
