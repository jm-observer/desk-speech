use crate::commands::history::DbSegmentDto;
use crate::lock_utils::mutex_lock;
use crate::AppState;
use log::info;

#[tauri::command]
pub fn list_segments(
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    info!("[list_segments] page={}, page_size={}", page, page_size);
    let db = mutex_lock(&state.db);
    let db = db.as_ref().ok_or("Database not initialized")?;
    crate::commands::history::list_segments(db, page, page_size)
}

#[tauri::command]
pub fn tail_segments(
    after_id: i64,
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    info!("[tail_segments] after_id={}, limit={}", after_id, limit);
    let db = mutex_lock(&state.db);
    let db = db.as_ref().ok_or("Database not initialized")?;
    crate::commands::history::tail_segments(db, after_id, limit)
}
