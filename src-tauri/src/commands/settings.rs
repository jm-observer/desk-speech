use crate::{AppState, CombinedSettings, ModelListResponse};

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Result<CombinedSettings, String> {
    crate::get_settings(state)
}

#[tauri::command]
pub fn apply_settings(new_settings: CombinedSettings, state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::apply_settings(new_settings, state)
}

#[tauri::command]
pub async fn list_llm_models(state: tauri::State<'_, AppState>) -> Result<ModelListResponse, String> {
    crate::list_llm_models(state).await
}
