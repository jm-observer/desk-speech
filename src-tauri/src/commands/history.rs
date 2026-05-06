use serde::Serialize;

use crate::db::{repository::SegmentRow, SpeechDatabase};

#[derive(Serialize, Clone)]
pub struct DbSegmentDto {
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

pub async fn list_segments(db: &SpeechDatabase, page: u32, page_size: u32) -> Result<Vec<DbSegmentDto>, String> {
    let rows = db.list_segments(page, page_size).await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(to_segment_dto).collect())
}

pub async fn tail_segments(db: &SpeechDatabase, after_id: i64, limit: u32) -> Result<Vec<DbSegmentDto>, String> {
    let rows = db.tail_segments(after_id, limit).await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(to_segment_dto).collect())
}

pub(crate) fn to_segment_dto(row: SegmentRow) -> DbSegmentDto {
    DbSegmentDto {
        id: row.id,
        segment_id: row.segment_id,
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
