pub mod repository;
pub mod schema;

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Local;
use deadpool_sqlite::{Config, Pool, Runtime};
use repository::{CorrectionRule, NewRule, NewSegment, OptimizeResultUpsert, SegmentRow, TranslateResultUpsert};

#[derive(Clone)]
pub struct SpeechDatabase {
    pool: Pool,
}

impl SpeechDatabase {
    pub async fn init(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create database directory: {}", parent.display()))?;
        }

        let config = Config::new(db_path);
        let pool = config
            .create_pool(Runtime::Tokio1)
            .context("failed to create sqlite pool")?;

        let conn = pool
            .get()
            .await
            .context("failed to get sqlite connection for migrations")?;
        let migration_result = conn
            .interact(|conn| schema::run_migrations(conn))
            .await
            .map_err(|e| anyhow::anyhow!("failed to join sqlite migration task: {e}"))?;
        migration_result?;

        Ok(Self { pool })
    }

    async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut deadpool_sqlite::rusqlite::Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.pool.get().await.context("failed to get sqlite connection")?;
        conn.interact(f)
            .await
            .map_err(|e| anyhow::anyhow!("failed to join sqlite interact task: {e}"))?
    }

    pub async fn ensure_global_scope(&self) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::ensure_global_scope(conn, &now))
            .await
    }

    pub async fn touch_global_scope_end(&self) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::touch_global_scope_end(conn, &now))
            .await
    }

    pub async fn upsert_segment(&self, segment: NewSegment) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::upsert_segment(conn, &segment, &now))
            .await
    }

    pub async fn mark_old_revisions_skipped(&self, latest_revision: i64) -> Result<()> {
        self.with_conn(move |conn| repository::mark_old_revisions_skipped(conn, latest_revision))
            .await
    }

    pub async fn update_optimize_status(&self, revision: i64, status: String) -> Result<()> {
        self.with_conn(move |conn| repository::update_optimize_status(conn, revision, &status))
            .await
    }

    pub async fn update_translate_status(&self, revision: i64, status: String) -> Result<()> {
        self.with_conn(move |conn| repository::update_translate_status(conn, revision, &status))
            .await
    }

    pub async fn upsert_optimize_result(&self, result: OptimizeResultUpsert) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::upsert_optimize_result(conn, &result, &now))
            .await
    }

    pub async fn upsert_translate_result(&self, result: TranslateResultUpsert) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::upsert_translate_result(conn, &result, &now))
            .await
    }

    pub async fn list_segments(&self, page: u32, page_size: u32) -> Result<Vec<SegmentRow>> {
        self.with_conn(move |conn| repository::list_segments(conn, page, page_size))
            .await
    }

    pub async fn get_next_segment_id(&self) -> Result<u64> {
        self.with_conn(|conn| repository::get_next_segment_id(conn)).await
    }

    pub async fn get_next_revision(&self) -> Result<u64> {
        self.with_conn(|conn| repository::get_next_revision(conn)).await
    }

    pub async fn tail_segments(&self, after_id: i64, limit: u32) -> Result<Vec<SegmentRow>> {
        self.with_conn(move |conn| repository::tail_segments(conn, after_id, limit))
            .await
    }

    pub async fn get_segment_by_revision(&self, revision: i64) -> Result<Option<SegmentRow>> {
        self.with_conn(move |conn| repository::get_segment_by_revision(conn, revision))
            .await
    }

    pub async fn upsert_rule(&self, rule: NewRule) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::upsert_rule(conn, &rule, &now))
            .await
    }

    pub async fn delete_rule(&self, rule_id: i64) -> Result<()> {
        self.with_conn(move |conn| repository::delete_rule(conn, rule_id)).await
    }

    pub async fn update_rule(&self, rule_id: i64, rule: NewRule) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::update_rule(conn, rule_id, &rule, &now))
            .await
    }

    pub async fn list_rules(&self) -> Result<Vec<CorrectionRule>> {
        self.with_conn(|conn| repository::list_rules(conn)).await
    }

    pub async fn bump_rule_version(&self, checksum: String) -> Result<i64> {
        let now = now_str();
        self.with_conn(move |conn| repository::bump_rule_version(conn, &checksum, &now))
            .await
    }

    pub async fn get_latest_rule_version(&self) -> Result<Option<(i64, String)>> {
        self.with_conn(|conn| repository::get_latest_rule_version(conn)).await
    }

    pub async fn upsert_setting(&self, key: String, value: String) -> Result<()> {
        let now = now_str();
        self.with_conn(move |conn| repository::upsert_setting(conn, &key, &value, &now))
            .await
    }

    pub async fn get_setting(&self, key: String) -> Result<Option<String>> {
        self.with_conn(move |conn| repository::get_setting(conn, &key)).await
    }
}

fn now_str() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
