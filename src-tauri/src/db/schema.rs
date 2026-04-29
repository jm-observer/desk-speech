use anyhow::{Context, Result};
use rusqlite::Connection;

pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    let sql = include_str!("../../migrations/0001_init.sql");
    conn.execute_batch(sql)
        .context("failed to run sqlite migration 0001_init.sql")?;
    Ok(())
}
