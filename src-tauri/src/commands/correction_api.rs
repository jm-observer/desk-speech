use crate::commands::correction::CorrectionRuleDto;
use crate::AppState;

#[tauri::command]
pub fn list_correction_rules(state: tauri::State<'_, AppState>) -> Result<Vec<CorrectionRuleDto>, String> {
    crate::list_correction_rules(state)
}

#[tauri::command]
pub fn create_correction_rule(
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    crate::create_correction_rule(source, target, priority, enabled, state)
}

#[tauri::command]
pub fn update_correction_rule(
    id: i64,
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    crate::update_correction_rule(id, source, target, priority, enabled, state)
}

#[tauri::command]
pub fn delete_correction_rule(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::delete_correction_rule(id, state)
}

#[tauri::command]
pub fn reload_correction_rules(state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::reload_correction_rules(state)
}
