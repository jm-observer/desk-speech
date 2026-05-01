use serde::Serialize;

use crate::db::{repository::SegmentRow, repository::SessionRow, SpeechDatabase};

#[derive(Serialize)]
pub struct DbSessionDto {
    pub id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub sample_rate: i64,
    pub channel_count: i64,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct DbSegmentDto {
    pub id: i64,
    pub session_id: String,
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

pub fn list_sessions(db: &SpeechDatabase, page: u32, page_size: u32) -> Result<Vec<DbSessionDto>, String> {
    let rows = db.list_sessions(page, page_size).map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(to_session_dto).collect())
}

pub fn list_session_segments(
    db: &SpeechDatabase,
    session_id: &str,
    page: u32,
    page_size: u32,
) -> Result<Vec<DbSegmentDto>, String> {
    ensure_session_exists(db, session_id)?;
    let rows = db
        .list_segments(session_id, page, page_size)
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(to_segment_dto).collect())
}

pub fn tail_session_segments(
    db: &SpeechDatabase,
    session_id: &str,
    after_id: i64,
    limit: u32,
) -> Result<Vec<DbSegmentDto>, String> {
    ensure_session_exists(db, session_id)?;
    let rows = db
        .tail_segments(session_id, after_id, limit)
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(to_segment_dto).collect())
}

fn ensure_session_exists(db: &SpeechDatabase, session_id: &str) -> Result<(), String> {
    let exists = db.session_exists(session_id).map_err(|e| e.to_string())?;
    if exists {
        Ok(())
    } else {
        Err(format!("session not found: {session_id}"))
    }
}

fn to_session_dto(row: SessionRow) -> DbSessionDto {
    DbSessionDto {
        id: row.id,
        started_at: row.started_at,
        ended_at: row.ended_at,
        sample_rate: row.sample_rate,
        channel_count: row.channel_count,
        created_at: row.created_at,
    }
}

fn to_segment_dto(row: SegmentRow) -> DbSegmentDto {
    DbSegmentDto {
        id: row.id,
        session_id: row.session_id,
        revision: row.revision,
        start_sec: row.start_sec,
        end_sec: row.end_sec,
        wall_start: row.wall_start,
        wall_end: row.wall_end,
        text_raw: row.text_raw,
        optimize_status: row.optimize_status,
        translate_status: row.translate_status,
        text_optimized: row.text_optimized,
        text_english: row.text_english,
        created_at: row.created_at,
    }
}
