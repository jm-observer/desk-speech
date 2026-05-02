use crate::commands::history::DbSegmentDto;
use crate::commands::history::DbSessionDto;
use crate::AppState;

#[tauri::command]
pub fn list_sessions(
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSessionDto>, String> {
    crate::list_sessions(page, page_size, state)
}

#[tauri::command]
pub fn list_session_segments(
    session_id: String,
    page: u32,
    page_size: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    crate::list_session_segments(session_id, page, page_size, state)
}

#[tauri::command]
pub fn tail_session_segments(
    session_id: String,
    after_id: i64,
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DbSegmentDto>, String> {
    crate::tail_session_segments(session_id, after_id, limit, state)
}
