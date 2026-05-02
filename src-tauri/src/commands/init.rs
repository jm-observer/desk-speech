use crate::{AppState, InitStatus};

#[tauri::command]
pub fn get_init_status(state: tauri::State<'_, AppState>) -> InitStatus {
    crate::get_init_status(state)
}
