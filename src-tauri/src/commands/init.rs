use crate::lock_utils::read_lock;
use crate::AppState;

#[derive(serde::Serialize, Clone)]
pub struct InitStatus {
    status: u8,
    error: String,
}

// Polled ~1/s by the frontend — intentionally no logging (would spam).
#[tauri::command]
pub fn get_init_status(state: tauri::State<'_, AppState>) -> Result<InitStatus, String> {
    let status = state.init_status.load(std::sync::atomic::Ordering::Relaxed);
    let error = read_lock(&state.init_error).clone();
    Ok(InitStatus { status, error })
}
