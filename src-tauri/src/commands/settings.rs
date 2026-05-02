use crate::settings::{CombinedSettings, ModelListResponse};
use crate::AppState;

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Result<CombinedSettings, String> {
    crate::settings::get_settings(state)
}

#[tauri::command]
pub async fn apply_settings(new_settings: CombinedSettings, state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::settings::apply_settings(new_settings, state).await
}

#[tauri::command]
pub async fn list_llm_models(state: tauri::State<'_, AppState>) -> Result<ModelListResponse, String> {
    crate::settings::list_llm_models(state).await
}
