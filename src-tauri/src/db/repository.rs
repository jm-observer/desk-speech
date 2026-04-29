use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};

#[derive(Clone, Debug)]
pub struct NewSegment {
    pub session_id: String,
    pub start_sec: f32,
    pub end_sec: f32,
    pub wall_start: String,
    pub wall_end: String,
    pub text_raw: String,
    pub text_corrected: String,
}

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

#[derive(Clone, Debug)]
pub struct SegmentRow {
    pub id: i64,
    pub session_id: String,
    pub start_sec: f32,
    pub end_sec: f32,
    pub wall_start: String,
    pub wall_end: String,
    pub text_raw: String,
    pub text_corrected: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct SessionRow {
    pub id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub sample_rate: i64,
    pub channel_count: i64,
    pub created_at: String,
}

pub fn create_session(conn: &Connection, session_id: &str, now: &str) -> Result<String> {
    conn.execute(
        "INSERT INTO sessions(id, started_at, created_at) VALUES (?1, ?2, ?2)",
        params![session_id, now],
    )
    .with_context(|| format!("failed to create session {session_id}"))?;
    Ok(session_id.to_string())
}

pub fn close_session(conn: &Connection, session_id: &str, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
        params![now, session_id],
    )
    .with_context(|| format!("failed to close session {session_id}"))?;
    Ok(())
}

pub fn session_exists(conn: &Connection, session_id: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM sessions WHERE id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .with_context(|| format!("failed to check session existence: {session_id}"))?;
    Ok(count > 0)
}

pub fn insert_segment(conn: &Connection, segment: &NewSegment, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO segments(session_id, start_sec, end_sec, wall_start, wall_end, text_raw, text_corrected, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            segment.session_id,
            segment.start_sec,
            segment.end_sec,
            segment.wall_start,
            segment.wall_end,
            segment.text_raw,
            segment.text_corrected,
            now,
        ],
    )
    .context("failed to insert segment")?;
    Ok(())
}

pub fn list_segments(conn: &Connection, session_id: &str, page: u32, page_size: u32) -> Result<Vec<SegmentRow>> {
    if page_size == 0 {
        return Err(anyhow!("page_size must be greater than 0"));
    }

    let offset = page as u64 * page_size as u64;
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, start_sec, end_sec, wall_start, wall_end, text_raw, text_corrected, created_at
             FROM segments
             WHERE session_id = ?1
             ORDER BY start_sec ASC
             LIMIT ?2 OFFSET ?3",
        )
        .context("failed to prepare list_segments statement")?;

    let mapped = stmt
        .query_map(params![session_id, page_size, offset], |row| {
            Ok(SegmentRow {
                id: row.get(0)?,
                session_id: row.get(1)?,
                start_sec: row.get(2)?,
                end_sec: row.get(3)?,
                wall_start: row.get(4)?,
                wall_end: row.get(5)?,
                text_raw: row.get(6)?,
                text_corrected: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .context("failed to query list_segments")?;

    let rows = mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect segments")?;
    Ok(rows)
}

pub fn list_sessions(conn: &Connection, page: u32, page_size: u32) -> Result<Vec<SessionRow>> {
    if page_size == 0 {
        return Err(anyhow!("page_size must be greater than 0"));
    }

    let offset = page as u64 * page_size as u64;
    let mut stmt = conn
        .prepare(
            "SELECT id, started_at, ended_at, sample_rate, channel_count, created_at
             FROM sessions
             ORDER BY started_at DESC
             LIMIT ?1 OFFSET ?2",
        )
        .context("failed to prepare list_sessions statement")?;

    let mapped = stmt
        .query_map(params![page_size, offset], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                started_at: row.get(1)?,
                ended_at: row.get(2)?,
                sample_rate: row.get(3)?,
                channel_count: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .context("failed to query list_sessions")?;

    let rows = mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect sessions")?;
    Ok(rows)
}

pub fn tail_segments(conn: &Connection, session_id: &str, after_id: i64, limit: u32) -> Result<Vec<SegmentRow>> {
    if limit == 0 {
        return Err(anyhow!("limit must be greater than 0"));
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, start_sec, end_sec, wall_start, wall_end, text_raw, text_corrected, created_at
             FROM segments
             WHERE session_id = ?1 AND id > ?2
             ORDER BY id ASC
             LIMIT ?3",
        )
        .context("failed to prepare tail_segments statement")?;
    let mapped = stmt
        .query_map(params![session_id, after_id, limit], |row| {
            Ok(SegmentRow {
                id: row.get(0)?,
                session_id: row.get(1)?,
                start_sec: row.get(2)?,
                end_sec: row.get(3)?,
                wall_start: row.get(4)?,
                wall_end: row.get(5)?,
                text_raw: row.get(6)?,
                text_corrected: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .context("failed to query tail_segments")?;

    let rows = mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect tail segments")?;
    Ok(rows)
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
        .collect::<rusqlite::Result<Vec<_>>>()
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
