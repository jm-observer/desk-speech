use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use log::{error, info};
use serde::{Deserialize, Serialize};

use crate::build_models;
use crate::db;
use crate::llm_client::{list_models as llm_list_models, model_cache_valid, CachedModels};
use crate::llm_settings::{validate_llm_settings, AutoCopyMode, LlmSettings};
use crate::lock_utils::{mutex_lock, read_lock, write_lock};
use crate::AppState;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct VadSettings {
    pub(crate) threshold: f32,
    pub(crate) min_silence_duration: f32,
    pub(crate) min_speech_duration: f32,
    pub(crate) max_speech_duration: f32,
    pub(crate) num_threads: i32,
}

impl Default for VadSettings {
    fn default() -> Self {
        Self {
            threshold: 0.2,
            min_silence_duration: 0.2,
            min_speech_duration: 0.2,
            max_speech_duration: 10.0,
            num_threads: 2,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CombinedSettings {
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    max_speech_duration: f32,
    num_threads: i32,
    provider_url: String,
    api_key: String,
    selected_model: String,
    optimize_prompt_template: String,
    translate_prompt_template: String,
    auto_copy_mode: AutoCopyMode,
}

#[derive(Serialize)]
pub(crate) struct ModelListResponse {
    models: Vec<String>,
}

pub(crate) fn get_settings(state: tauri::State<'_, AppState>) -> Result<CombinedSettings, String> {
    info!("[get_settings]");
    let settings = Arc::clone(&state.settings);
    let llm_settings = Arc::clone(&state.llm_settings);
    let vad = read_lock(&settings).clone();
    let llm = read_lock(&llm_settings).clone();
    Ok(CombinedSettings {
        threshold: vad.threshold,
        min_silence_duration: vad.min_silence_duration,
        min_speech_duration: vad.min_speech_duration,
        max_speech_duration: vad.max_speech_duration,
        num_threads: vad.num_threads,
        provider_url: llm.provider_url,
        api_key: llm.api_key,
        selected_model: llm.selected_model,
        optimize_prompt_template: llm.optimize_prompt_template,
        translate_prompt_template: llm.translate_prompt_template,
        auto_copy_mode: llm.auto_copy_mode,
    })
}

fn validate_settings(s: &VadSettings) -> Result<(), String> {
    if s.threshold <= 0.0 || s.threshold >= 1.0 {
        return Err("threshold must be between 0.0 and 1.0 (exclusive)".to_string());
    }
    if s.min_silence_duration < 0.0 {
        return Err("min_silence_duration must be >= 0".to_string());
    }
    if s.min_speech_duration < 0.0 {
        return Err("min_speech_duration must be >= 0".to_string());
    }
    if s.max_speech_duration <= 0.0 {
        return Err("max_speech_duration must be > 0".to_string());
    }
    if s.num_threads < 1 || s.num_threads > 16 {
        return Err("num_threads must be between 1 and 16".to_string());
    }
    Ok(())
}

pub(crate) async fn apply_settings(
    new_settings: CombinedSettings,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!("[apply_settings]");
    let settings_arc = Arc::clone(&state.settings);
    let llm_settings_arc = Arc::clone(&state.llm_settings);
    let llm_models_cache_arc = Arc::clone(&state.llm_models_cache);
    let db_arc = Arc::clone(&state.db);
    if state.recording.load(Ordering::SeqCst) {
        return Err("Cannot change settings while recording".to_string());
    }
    let init = state.init_status.load(Ordering::Relaxed);
    if init == 0 {
        return Err("Models are still loading, please wait".to_string());
    }

    let new_vad_settings = VadSettings {
        threshold: new_settings.threshold,
        min_silence_duration: new_settings.min_silence_duration,
        min_speech_duration: new_settings.min_speech_duration,
        max_speech_duration: new_settings.max_speech_duration,
        num_threads: new_settings.num_threads,
    };
    let new_llm_settings = LlmSettings {
        provider_url: new_settings.provider_url,
        api_key: new_settings.api_key,
        selected_model: new_settings.selected_model,
        optimize_prompt_template: new_settings.optimize_prompt_template,
        translate_prompt_template: new_settings.translate_prompt_template,
        discard_prompt_template: String::new(),
        auto_copy_mode: new_settings.auto_copy_mode,
    };

    validate_settings(&new_vad_settings)?;
    validate_llm_settings(&new_llm_settings)?;

    {
        let current_vad = read_lock(&settings_arc);
        let current_llm = read_lock(&llm_settings_arc);
        if *current_vad == new_vad_settings && *current_llm == new_llm_settings {
            return Ok(());
        }
    }

    state.init_status.store(0, Ordering::Relaxed);
    *write_lock(&settings_arc) = new_vad_settings.clone();
    *write_lock(&llm_settings_arc) = new_llm_settings.clone();
    *write_lock(&llm_models_cache_arc) = None;

    let db = {
        let guard = mutex_lock(&db_arc);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    {
        db.upsert_setting("llm.provider_url".to_string(), new_llm_settings.provider_url.clone())
            .await
            .map_err(|e| e.to_string())?;
        db.upsert_setting("llm.api_key".to_string(), new_llm_settings.api_key.clone())
            .await
            .map_err(|e| e.to_string())?;
        db.upsert_setting(
            "llm.selected_model".to_string(),
            new_llm_settings.selected_model.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        db.upsert_setting(
            "llm.optimize_prompt_template".to_string(),
            new_llm_settings.optimize_prompt_template.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        db.upsert_setting(
            "llm.translate_prompt_template".to_string(),
            new_llm_settings.translate_prompt_template.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        db.upsert_setting(
            "llm.auto_copy_mode".to_string(),
            match new_llm_settings.auto_copy_mode {
                AutoCopyMode::Off => "off",
                AutoCopyMode::English => "english",
                AutoCopyMode::OptimizedZh => "optimized_zh",
            }
            .to_string(),
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    let recognizer_arc = Arc::clone(&state.recognizer);
    let vad_arc = Arc::clone(&state.vad);
    let init_status = Arc::clone(&state.init_status);
    let init_error = Arc::clone(&state.init_error);
    let init_num_threads = Arc::clone(&state.num_threads);

    tauri::async_runtime::spawn(async move {
        info!("[apply_settings] rebuilding models...");
        let join = tauri::async_runtime::spawn_blocking(move || build_models(&new_vad_settings));
        match join.await {
            Ok(Ok((rec, vad, threads))) => {
                info!("[apply_settings] models rebuilt, num_threads={threads}");
                {
                    let mut r = write_lock(&recognizer_arc);
                    *r = Some(rec);
                }
                {
                    let mut v = write_lock(&vad_arc);
                    *v = Some(vad);
                }
                init_num_threads.store(threads, Ordering::Relaxed);
                init_status.store(1, Ordering::Relaxed);
            }
            Ok(Err(err)) => {
                error!("[apply_settings] rebuild failed: {err}");
                {
                    let mut init_err = write_lock(&init_error);
                    *init_err = err;
                }
                init_status.store(2, Ordering::Relaxed);
            }
            Err(err) => {
                error!("[apply_settings] join failed: {err}");
                {
                    let mut init_err = write_lock(&init_error);
                    *init_err = "Internal error: settings task join failed".to_string();
                }
                init_status.store(2, Ordering::Relaxed);
            }
        }
    });

    Ok(())
}

pub(crate) async fn list_llm_models(state: tauri::State<'_, AppState>) -> Result<ModelListResponse, String> {
    info!("[list_llm_models]");
    let settings = read_lock(&state.llm_settings).clone();
    validate_llm_settings(&settings)?;

    if let Some(cache) = read_lock(&state.llm_models_cache).as_ref() {
        if model_cache_valid(cache) {
            return Ok(ModelListResponse {
                models: cache.models.clone(),
            });
        }
    }

    let fetched = llm_list_models(&settings).await?;
    *write_lock(&state.llm_models_cache) = Some(CachedModels {
        fetched_at: Instant::now(),
        models: fetched.clone(),
    });
    Ok(ModelListResponse { models: fetched })
}

pub(crate) async fn load_llm_settings_from_db(db: &db::SpeechDatabase) -> LlmSettings {
    let mut settings = LlmSettings::default();

    if let Ok(Some(v)) = db.get_setting("llm.provider_url".to_string()).await {
        settings.provider_url = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.api_key".to_string()).await {
        settings.api_key = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.selected_model".to_string()).await {
        settings.selected_model = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.optimize_prompt_template".to_string()).await {
        settings.optimize_prompt_template = v;
    } else if let Ok(Some(v)) = db.get_setting("llm.prompt_template".to_string()).await {
        settings.optimize_prompt_template = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.translate_prompt_template".to_string()).await {
        settings.translate_prompt_template = v;
    }
    if let Ok(Some(v)) = db.get_setting("llm.auto_copy_mode".to_string()).await {
        settings.auto_copy_mode = match v.as_str() {
            "off" => AutoCopyMode::Off,
            "optimized_zh" => AutoCopyMode::OptimizedZh,
            _ => AutoCopyMode::English,
        };
    } else if let Ok(Some(v)) = db.get_setting("llm.auto_copy".to_string()).await {
        settings.auto_copy_mode = if v == "false" || v == "0" {
            AutoCopyMode::Off
        } else {
            AutoCopyMode::English
        };
    }

    settings
}
