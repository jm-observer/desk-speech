use crate::lock_utils::read_lock;
use crate::AppState;
use log::info;

#[derive(serde::Serialize, Clone)]
pub struct InitStatus {
    status: u8,
    error: String,
    num_threads: u32,
}

#[tauri::command]
pub fn get_init_status(state: tauri::State<'_, AppState>) -> Result<InitStatus, String> {
    info!("[get_init_status]");
    let status = state.init_status.load(std::sync::atomic::Ordering::Relaxed);
    let error = read_lock(&state.init_error).clone();
    let num_threads = state.num_threads.load(std::sync::atomic::Ordering::Relaxed);
    Ok(InitStatus {
        status,
        error,
        num_threads,
    })
}
