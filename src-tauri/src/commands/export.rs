use crate::AppState;

#[tauri::command]
pub fn save_segment_as_wav(
    path: String,
    start: f32,
    end: f32,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    crate::save_segment_as_wav(path, start, end, state)
}

#[tauri::command]
pub fn save_all_audio(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::save_all_audio(path, state)
}

#[tauri::command]
pub fn get_recorded_audio_path(state: tauri::State<'_, AppState>) -> Result<String, String> {
    crate::get_recorded_audio_path(state)
}

#[tauri::command]
pub fn export_srt(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::export_srt(path, state)
}

#[tauri::command]
pub fn copy_text_to_clipboard(app: tauri::AppHandle, text: String) -> Result<(), String> {
    crate::copy_text_to_clipboard(app, text)
}
