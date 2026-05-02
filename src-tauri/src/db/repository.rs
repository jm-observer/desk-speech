use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};

pub(crate) const GLOBAL_SCOPE_ID: &str = "global";

#[derive(Clone, Debug)]
pub struct NewSegment {
    pub segment_id: u64,
    pub revision: i64,
    pub start_sec: f32,
    pub end_sec: f32,
    pub wall_start: String,
    pub wall_end: String,
    pub text_raw: String,
}

#[derive(Clone, Debug)]
pub struct OptimizeResultUpsert {
    pub revision: i64,
    pub text_optimized: Option<String>,
    pub optimize_error: Option<String>,
    pub optimize_started_at: Option<String>,
    pub optimize_finished_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TranslateResultUpsert {
    pub revision: i64,
    pub text_english: Option<String>,
    pub translate_error: Option<String>,
    pub translate_started_at: Option<String>,
    pub translate_finished_at: Option<String>,
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
    pub segment_id: u64,
    pub revision: i64,
    pub start_sec: f32,
    pub end_sec: f32,
    pub wall_start: String,
    pub wall_end: String,
    pub text_raw: String,
    pub optimize_status: String,
    pub translate_status: String,
    pub text_optimized: Option<String>,
    pub text_english: Option<String>,
    pub created_at: String,
}

pub fn ensure_global_scope(conn: &Connection, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions(id, started_at, created_at)
         VALUES (?1, ?2, ?2)
         ON CONFLICT(id) DO NOTHING",
        params![GLOBAL_SCOPE_ID, now],
    )
    .context("failed to ensure global scope")?;
    Ok(())
}

pub fn touch_global_scope_end(conn: &Connection, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
        params![now, GLOBAL_SCOPE_ID],
    )
    .context("failed to update global scope end time")?;
    Ok(())
}

pub fn global_scope_exists(conn: &Connection) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM sessions WHERE id = ?1",
            params![GLOBAL_SCOPE_ID],
            |row| row.get(0),
        )
        .context("failed to check global scope existence")?;
    Ok(count > 0)
}

pub fn upsert_segment(conn: &Connection, segment: &NewSegment, now: &str) -> Result<()> {
    let sql = "INSERT INTO asr_raw_records(session_id, segment_id, revision, start_sec, end_sec, wall_start, wall_end, text_raw, opt_status, optimize_status, translate_status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 'pending', 'blocked', ?9)
         ON CONFLICT(session_id, segment_id) DO UPDATE SET
            revision = excluded.revision,
            start_sec = excluded.start_sec,
            end_sec = excluded.end_sec,
            wall_start = excluded.wall_start,
            wall_end = excluded.wall_end,
            text_raw = excluded.text_raw,
            created_at = excluded.created_at";
    conn.execute(
        sql,
        params![
            GLOBAL_SCOPE_ID,
            segment.segment_id,
            segment.revision,
            segment.start_sec,
            segment.end_sec,
            segment.wall_start,
            segment.wall_end,
            segment.text_raw,
            now,
        ],
    )
    .with_context(|| {
        format!(
            "failed to upsert segment segment_id={}, revision={}",
            segment.segment_id, segment.revision
        )
    })?;
    Ok(())
}

pub fn update_optimize_status(conn: &Connection, revision: i64, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE asr_raw_records SET optimize_status = ?1 WHERE session_id = ?2 AND revision = ?3",
        params![status, GLOBAL_SCOPE_ID, revision],
    )
    .with_context(|| format!("failed to update optimize_status for revision {revision}"))?;
    Ok(())
}

pub fn update_translate_status(conn: &Connection, revision: i64, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE asr_raw_records SET translate_status = ?1 WHERE session_id = ?2 AND revision = ?3",
        params![status, GLOBAL_SCOPE_ID, revision],
    )
    .with_context(|| format!("failed to update translate_status for revision {revision}"))?;
    Ok(())
}

pub fn mark_old_revisions_skipped(conn: &Connection, latest_revision: i64) -> Result<()> {
    conn.execute(
        "UPDATE asr_raw_records
         SET optimize_status = 'failed', translate_status = 'blocked'
         WHERE session_id = ?1 AND revision < ?2 AND optimize_status IN ('pending', 'running')",
        params![GLOBAL_SCOPE_ID, latest_revision],
    )
    .context("failed to mark skipped revisions")?;
    Ok(())
}

pub fn upsert_optimize_result(conn: &Connection, result: &OptimizeResultUpsert, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO asr_llm_results(session_id, revision, text_optimized, optimize_error, optimize_started_at, optimize_finished_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(session_id, revision) DO UPDATE SET
            text_optimized = excluded.text_optimized,
            optimize_error = excluded.optimize_error,
            optimize_started_at = excluded.optimize_started_at,
            optimize_finished_at = excluded.optimize_finished_at,
            created_at = excluded.created_at",
        params![
            GLOBAL_SCOPE_ID,
            result.revision,
            result.text_optimized,
            result.optimize_error,
            result.optimize_started_at,
            result.optimize_finished_at,
            now,
        ],
    )
    .context("failed to upsert optimize result")?;
    Ok(())
}

pub fn upsert_translate_result(conn: &Connection, result: &TranslateResultUpsert, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO asr_llm_results(session_id, revision, text_english, translate_error, translate_started_at, translate_finished_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(session_id, revision) DO UPDATE SET
            text_english = excluded.text_english,
            translate_error = excluded.translate_error,
            translate_started_at = excluded.translate_started_at,
            translate_finished_at = excluded.translate_finished_at,
            created_at = excluded.created_at",
        params![
            GLOBAL_SCOPE_ID,
            result.revision,
            result.text_english,
            result.translate_error,
            result.translate_started_at,
            result.translate_finished_at,
            now,
        ],
    )
    .context("failed to upsert translate result")?;
    Ok(())
}

pub fn get_last_segment(conn: &Connection) -> Result<Option<SegmentRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.segment_id, r.session_id, r.revision, r.start_sec, r.end_sec, r.wall_start, r.wall_end, r.text_raw,
                    r.optimize_status, r.translate_status, l.text_optimized, l.text_english, r.created_at
             FROM asr_raw_records r
             LEFT JOIN asr_llm_results l ON l.session_id = r.session_id AND l.revision = r.revision
             WHERE session_id = ?1
             ORDER BY r.id DESC
             LIMIT 1",
        )
        .context("failed to prepare get_last_segment statement")?;

    let mut rows = stmt
        .query(params![GLOBAL_SCOPE_ID])
        .context("failed to query last segment")?;
    if let Some(row) = rows.next().context("failed to read last segment row")? {
        Ok(Some(SegmentRow {
            id: row.get(0)?,
            segment_id: row.get(1)?,
            revision: row.get(3)?,
            start_sec: row.get(4)?,
            end_sec: row.get(5)?,
            wall_start: row.get(6)?,
            wall_end: row.get(7)?,
            text_raw: row.get(8)?,
            optimize_status: row.get(9)?,
            translate_status: row.get(10)?,
            text_optimized: row.get(11)?,
            text_english: row.get(12)?,
            created_at: row.get(13)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn get_next_segment_id(conn: &Connection) -> Result<u64> {
    let next_segment_id: u64 = conn
        .query_row(
            "SELECT COALESCE(MAX(segment_id), 0) + 1 FROM asr_raw_records WHERE session_id = ?1",
            params![GLOBAL_SCOPE_ID],
            |row| row.get(0),
        )
        .context("failed to query next segment_id")?;
    Ok(next_segment_id)
}

pub fn get_next_revision(conn: &Connection) -> Result<u64> {
    let next_revision: u64 = conn
        .query_row(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM asr_raw_records WHERE session_id = ?1",
            params![GLOBAL_SCOPE_ID],
            |row| row.get(0),
        )
        .context("failed to query next revision")?;
    Ok(next_revision)
}

pub fn list_segments(conn: &Connection, page: u32, page_size: u32) -> Result<Vec<SegmentRow>> {
    if page_size == 0 {
        return Err(anyhow!("page_size must be greater than 0"));
    }

    let offset = page as u64 * page_size as u64;
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.segment_id, r.session_id, r.revision, r.start_sec, r.end_sec, r.wall_start, r.wall_end, r.text_raw,
                    r.optimize_status, r.translate_status, l.text_optimized, l.text_english, r.created_at
             FROM asr_raw_records r
             LEFT JOIN asr_llm_results l ON l.session_id = r.session_id AND l.revision = r.revision
             WHERE r.session_id = ?1
             ORDER BY r.start_sec ASC
             LIMIT ?2 OFFSET ?3",
        )
        .context("failed to prepare list_segments statement")?;

    let mapped = stmt
        .query_map(params![GLOBAL_SCOPE_ID, page_size, offset], |row| {
            Ok(SegmentRow {
                id: row.get(0)?,
                segment_id: row.get(1)?,
                revision: row.get(3)?,
                start_sec: row.get(4)?,
                end_sec: row.get(5)?,
                wall_start: row.get(6)?,
                wall_end: row.get(7)?,
                text_raw: row.get(8)?,
                optimize_status: row.get(9)?,
                translate_status: row.get(10)?,
                text_optimized: row.get(11)?,
                text_english: row.get(12)?,
                created_at: row.get(13)?,
            })
        })
        .context("failed to query list_segments")?;

    let rows = mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect segments")?;
    Ok(rows)
}

pub fn tail_segments(conn: &Connection, after_id: i64, limit: u32) -> Result<Vec<SegmentRow>> {
    if limit == 0 {
        return Err(anyhow!("limit must be greater than 0"));
    }
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.segment_id, r.session_id, r.revision, r.start_sec, r.end_sec, r.wall_start, r.wall_end, r.text_raw,
                    r.optimize_status, r.translate_status, l.text_optimized, l.text_english, r.created_at
             FROM asr_raw_records r
             LEFT JOIN asr_llm_results l ON l.session_id = r.session_id AND l.revision = r.revision
             WHERE r.session_id = ?1 AND r.id > ?2
             ORDER BY r.id ASC
             LIMIT ?3",
        )
        .context("failed to prepare tail_segments statement")?;
    let mapped = stmt
        .query_map(params![GLOBAL_SCOPE_ID, after_id, limit], |row| {
            Ok(SegmentRow {
                id: row.get(0)?,
                segment_id: row.get(1)?,
                revision: row.get(3)?,
                start_sec: row.get(4)?,
                end_sec: row.get(5)?,
                wall_start: row.get(6)?,
                wall_end: row.get(7)?,
                text_raw: row.get(8)?,
                optimize_status: row.get(9)?,
                translate_status: row.get(10)?,
                text_optimized: row.get(11)?,
                text_english: row.get(12)?,
                created_at: row.get(13)?,
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
