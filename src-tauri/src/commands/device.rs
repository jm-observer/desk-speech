use crate::{AppState, InputDevice};

#[tauri::command]
pub fn list_input_devices() -> Result<Vec<InputDevice>, String> {
    crate::list_input_devices()
}

#[tauri::command]
pub fn set_input_device(device_name: Option<String>, state: tauri::State<'_, AppState>) -> Result<(), String> {
    crate::set_input_device(device_name, state)
}

#[tauri::command]
pub fn get_selected_device(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    crate::get_selected_device(state)
}
