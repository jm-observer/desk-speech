use crate::commands::correction::CorrectionRuleDto;
use crate::lock_utils::mutex_lock;
use crate::AppState;
use log::info;

#[tauri::command]
pub async fn list_correction_rules(state: tauri::State<'_, AppState>) -> Result<Vec<CorrectionRuleDto>, String> {
    info!("[list_correction_rules]");
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::correction::list_correction_rules(&db).await
}

#[tauri::command]
pub async fn create_correction_rule(
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!(
        "[create_correction_rule] source={}, target={}, priority={}, enabled={}",
        source, target, priority, enabled
    );
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::correction::create_correction_rule(
        &db,
        &state.correction_engine,
        source,
        target,
        priority,
        enabled,
    )
    .await
}

#[tauri::command]
pub async fn update_correction_rule(
    id: i64,
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!(
        "[update_correction_rule] id={}, source={}, target={}, priority={}, enabled={}",
        id, source, target, priority, enabled
    );
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::correction::update_correction_rule(
        &db,
        &state.correction_engine,
        id,
        source,
        target,
        priority,
        enabled,
    )
    .await
}

#[tauri::command]
pub async fn delete_correction_rule(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[delete_correction_rule] id={}", id);
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::correction::delete_correction_rule(&db, &state.correction_engine, id).await
}

#[tauri::command]
pub async fn reload_correction_rules(state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[reload_correction_rules]");
    let db = {
        let guard = mutex_lock(&state.db);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    crate::commands::correction::reload_correction_rules(&db, &state.correction_engine).await
}
