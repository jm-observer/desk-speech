use anyhow::{Context, Result};
use rusqlite::Connection;

pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    let sql_0001 = include_str!("../../migrations/0001_init.sql");
    conn.execute_batch(sql_0001)
        .context("failed to run sqlite migration 0001_init.sql")?;
    if !column_exists(conn, "asr_raw_records", "optimize_status")? {
        let sql_0002 = include_str!("../../migrations/0002_split_optimize_translate.sql");
        conn.execute_batch(sql_0002)
            .context("failed to run sqlite migration 0002_split_optimize_translate.sql")?;
    }
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("failed to prepare table info query for {table}"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("failed to query table info for {table}"))?;
    while let Some(row) = rows.next().context("failed to read table info row")? {
        let name: String = row.get(1).context("failed to get table column name")?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}
