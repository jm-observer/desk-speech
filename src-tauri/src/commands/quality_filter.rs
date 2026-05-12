use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::config::quality_filter::{ConfigValidationError, QualityFilterConfig};
use crate::lock_utils::{read_lock, write_lock};
use crate::AppState;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QualityFilterConfigResponse {
    pub llm_prompt_template: String,
    pub discard_confidence_threshold: f32,
    pub silence_window_ms: u64,
    pub repeat_ratio_threshold: f32,
    pub enabled: bool,
    pub version: u32,
}

impl From<QualityFilterConfig> for QualityFilterConfigResponse {
    fn from(config: QualityFilterConfig) -> Self {
        Self {
            llm_prompt_template: config.llm_prompt_template,
            discard_confidence_threshold: config.discard_confidence_threshold,
            silence_window_ms: config.silence_window_ms,
            repeat_ratio_threshold: config.repeat_ratio_threshold,
            enabled: config.enabled,
            version: config.version,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SaveQualityFilterConfigPayload {
    pub llm_prompt_template: String,
    pub discard_confidence_threshold: f32,
    pub silence_window_ms: u64,
    pub repeat_ratio_threshold: f32,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ValidationErrorsResponse {
    pub errors: Vec<ConfigValidationError>,
}

#[tauri::command]
pub async fn get_quality_filter_config(
    state: tauri::State<'_, AppState>,
) -> Result<QualityFilterConfigResponse, String> {
    info!("[get_quality_filter_config]");
    let config = read_lock(&state.quality_filter_config).clone();
    Ok(config.into())
}

#[tauri::command]
pub async fn save_quality_filter_config(
    payload: SaveQualityFilterConfigPayload,
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    info!("[save_quality_filter_config]");

    // Build config from payload
    let config = QualityFilterConfig {
        llm_prompt_template: payload.llm_prompt_template,
        discard_confidence_threshold: payload.discard_confidence_threshold,
        silence_window_ms: payload.silence_window_ms,
        repeat_ratio_threshold: payload.repeat_ratio_threshold,
        enabled: payload.enabled,
        ..QualityFilterConfig::default()
    };

    // Validate
    if let Err(errors) = config.validate() {
        warn!("[save_quality_filter_config] validation failed: {:?}", errors);
        let _ = app_handle.emit(
            "quality_filter_config_validation_error",
            ValidationErrorsResponse { errors },
        );
        return Err("Configuration validation failed".to_string());
    }

    // Update in-memory config
    {
        let mut current = write_lock(&state.quality_filter_config);
        *current = config.clone();
    }

    // Persist to database
    let db = {
        let guard = crate::lock_utils::mutex_lock(&state.db);
        guard.clone().ok_or("Database not initialized")?
    };
    crate::settings::save_quality_filter_config_to_db(&db, &config).await?;

    // Broadcast config updated event
    let _ = app_handle.emit("quality_filter_config_updated", config.clone());

    info!("[save_quality_filter_config] saved successfully");
    Ok(())
}

#[tauri::command]
pub async fn reset_quality_filter_config(
    state: tauri::State<'_, AppState>,
) -> Result<QualityFilterConfigResponse, String> {
    info!("[reset_quality_filter_config]");
    let default_config = QualityFilterConfig::default();

    {
        let mut current = write_lock(&state.quality_filter_config);
        *current = default_config.clone();
    }

    let db = {
        let guard = crate::lock_utils::mutex_lock(&state.db);
        guard.clone().ok_or("Database not initialized")?
    };
    crate::settings::save_quality_filter_config_to_db(&db, &default_config).await?;

    Ok(default_config.into())
}
