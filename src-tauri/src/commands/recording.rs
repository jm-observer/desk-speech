use crate::{AppState, RecordingState};

#[tauri::command]
pub fn start_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::start_recording(app, state)
}

#[tauri::command]
pub fn stop_recording(state: tauri::State<'_, AppState>) {
    crate::stop_recording(state);
}

#[tauri::command]
pub fn clear_results(state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::clear_results(state)
}

#[tauri::command]
pub fn get_recording_state(state: tauri::State<'_, AppState>) -> Result<RecordingState, String> {
    crate::get_recording_state(state)
}
