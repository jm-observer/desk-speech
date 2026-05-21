use anyhow::{Context, Result};
use deadpool_sqlite::rusqlite::{params, Connection};

#[derive(Clone, Debug)]
pub struct NewRule {
    pub source: String,
    pub target: String,
    pub enabled: bool,
    pub priority: i32,
}

#[derive(Clone, Debug)]
pub struct CorrectionRule {
    pub id: i64,
    pub source: String,
    pub target: String,
    pub enabled: bool,
    pub priority: i32,
    pub updated_at: String,
}

pub fn upsert_rule(conn: &Connection, rule: &NewRule, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO correction_rules(source, target, enabled, priority, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(source, target)
         DO UPDATE SET enabled = excluded.enabled, priority = excluded.priority, updated_at = excluded.updated_at",
        params![rule.source, rule.target, rule.enabled as i32, rule.priority, now],
    )
    .context("failed to upsert correction rule")?;
    Ok(())
}

pub fn delete_rule(conn: &Connection, rule_id: i64) -> Result<()> {
    conn.execute("DELETE FROM correction_rules WHERE id = ?1", params![rule_id])
        .with_context(|| format!("failed to delete rule {rule_id}"))?;
    Ok(())
}

pub fn update_rule(conn: &Connection, rule_id: i64, rule: &NewRule, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE correction_rules
         SET source = ?1, target = ?2, enabled = ?3, priority = ?4, updated_at = ?5
         WHERE id = ?6",
        params![
            rule.source,
            rule.target,
            rule.enabled as i32,
            rule.priority,
            now,
            rule_id
        ],
    )
    .with_context(|| format!("failed to update rule {rule_id}"))?;
    Ok(())
}

pub fn list_rules(conn: &Connection) -> Result<Vec<CorrectionRule>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source, target, enabled, priority, updated_at
             FROM correction_rules
             ORDER BY priority ASC, id ASC",
        )
        .context("failed to prepare list_rules statement")?;

    let mapped = stmt
        .query_map([], |row| {
            Ok(CorrectionRule {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                enabled: row.get::<_, i32>(3)? == 1,
                priority: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .context("failed to query rules")?;

    let rows = mapped
        .collect::<deadpool_sqlite::rusqlite::Result<Vec<_>>>()
        .context("failed to collect rules")?;
    Ok(rows)
}

pub fn bump_rule_version(conn: &Connection, checksum: &str, now: &str) -> Result<i64> {
    let next_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM correction_rule_versions",
            [],
            |row| row.get(0),
        )
        .context("failed to get next rule version")?;

    conn.execute(
        "INSERT INTO correction_rule_versions(version, checksum, created_at) VALUES (?1, ?2, ?3)",
        params![next_version, checksum, now],
    )
    .context("failed to insert rule version")?;

    Ok(next_version)
}

pub fn get_latest_rule_version(conn: &Connection) -> Result<Option<(i64, String)>> {
    let mut stmt = conn
        .prepare("SELECT version, checksum FROM correction_rule_versions ORDER BY version DESC LIMIT 1")
        .context("failed to prepare get_latest_rule_version")?;

    let mut rows = stmt.query([]).context("failed to query latest rule version")?;
    if let Some(row) = rows.next().context("failed to read latest rule version row")? {
        let version = row.get(0)?;
        let checksum = row.get(1)?;
        Ok(Some((version, checksum)))
    } else {
        Ok(None)
    }
}

pub fn upsert_setting(conn: &Connection, key: &str, value: &str, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO app_settings(key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now],
    )
    .with_context(|| format!("failed to upsert setting: {key}"))?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare("SELECT value FROM app_settings WHERE key = ?1")
        .with_context(|| format!("failed to prepare get_setting for {key}"))?;
    let mut rows = stmt.query(params![key]).context("failed to query setting")?;
    if let Some(row) = rows.next().context("failed to read setting row")? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}
