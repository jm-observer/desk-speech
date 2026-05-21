pub mod repository;
pub mod schema;

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Local;
use deadpool_sqlite::{Config, Pool, Runtime};
use repository::{CorrectionRule, NewRule};

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
