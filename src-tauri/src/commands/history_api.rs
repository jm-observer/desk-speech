use crate::commands::history::DbSegmentDto;
use crate::lock_utils::mutex_lock;
use crate::{remove_segment_from_memory, AppState};
use log::info;

#[tauri::command]
pub async fn list_segments(
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    info!("[list_segments] page={}, page_size={}", page, page_size);
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::history::list_segments(&db, page, page_size).await
}

#[tauri::command]
pub async fn tail_segments(
    after_id: i64,
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    info!("[tail_segments] after_id={}, limit={}", after_id, limit);
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::history::tail_segments(&db, after_id, limit).await
}

#[tauri::command]
pub async fn delete_segment(segment_id: u64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[delete_segment] segment_id={}", segment_id);
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::history::delete_segment(&db, segment_id).await?;
    remove_segment_from_memory(&state.segments, segment_id);
    Ok(())
}
