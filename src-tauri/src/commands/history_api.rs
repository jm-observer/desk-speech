use crate::commands::history::DbSegmentDto;
use crate::commands::history::DbSessionDto;
use crate::lock_utils::mutex_lock;
use crate::AppState;
use log::info;

#[tauri::command]
pub fn list_sessions(
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSessionDto>, String> {
    info!("[list_sessions] page={}, page_size={}", page, page_size);
    let db = mutex_lock(&state.db);
    let db = db.as_ref().ok_or("Database not initialized")?;
    crate::commands::history::list_sessions(db, page, page_size)
}

#[tauri::command]
pub fn list_session_segments(
    session_id: String,
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    info!(
        "[list_session_segments] session_id={}, page={}, page_size={}",
        session_id, page, page_size
    );
    let db = mutex_lock(&state.db);
    let db = db.as_ref().ok_or("Database not initialized")?;
    crate::commands::history::list_session_segments(db, &session_id, page, page_size)
}

#[tauri::command]
pub fn tail_session_segments(
    session_id: String,
    after_id: i64,
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    info!(
        "[tail_session_segments] session_id={}, after_id={}, limit={}",
        session_id, after_id, limit
    );
    let db = mutex_lock(&state.db);
    let db = db.as_ref().ok_or("Database not initialized")?;
    crate::commands::history::tail_session_segments(db, &session_id, after_id, limit)
}
