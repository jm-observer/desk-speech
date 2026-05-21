pub mod repository;
pub mod schema;

use std::path::Path;

use anyhow::{Context, Result};
use chrono::Local;
use deadpool_sqlite::{Config, Pool, Runtime};

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
