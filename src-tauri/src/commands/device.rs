use cpal::traits::{DeviceTrait, HostTrait};
use log::info;

use crate::AppState;

#[derive(serde::Serialize, Clone)]
pub struct InputDevice {
    name: String,
    is_default: bool,
}

#[tauri::command]
pub fn list_input_devices() -> Result<Vec<InputDevice>, String> {
    info!("[list_input_devices]");
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let devices: Vec<InputDevice> = host
        .input_devices()
        .map_err(|e| format!("Cannot enumerate devices: {e}"))?
        .filter_map(|d| {
            let name = d.name().ok()?;
            Some(InputDevice {
                is_default: name == default_name,
                name,
            })
        })
        .collect();

    Ok(devices)
}

#[tauri::command]
pub fn set_input_device(device_name: Option<String>, state: tauri::State<'_, AppState>) -> Result<(), String> {
    info!("[set_input_device] device_name={:?}", device_name);
    if state.recording.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Cannot change device while recording".to_string());
    }
    *state.selected_device.blocking_write() = device_name;
    Ok(())
}

#[tauri::command]
pub fn get_selected_device(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    info!("[get_selected_device]");
    Ok(state.selected_device.blocking_read().clone())
}
