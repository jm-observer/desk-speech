use std::sync::Arc;

use log::info;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::llm_settings::{AutoCopyMode, LlmSettings};
use crate::lock_utils::{mutex_lock, read_lock, write_lock};
use crate::AppState;

/// Only client-side language pick (sent in the WS `hello.language` field).
/// Everything else (model, threshold, gap, prompts, vLLM URL) lives on the
/// orchestrator and is edited from the GB10 web console.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct VadSettings {
    pub(crate) asr_language: String,
}

impl Default for VadSettings {
    fn default() -> Self {
        Self {
            asr_language: "zh".to_string(),
        }
    }
}

/// Combined settings DTO exchanged with the frontend — the two real choices
/// the desktop client makes: which ASR language to request, and whether to
/// auto-copy the optimized Chinese / English translation to the clipboard.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CombinedSettings {
    asr_language: String,
    auto_copy_mode: AutoCopyMode,
}

pub(crate) fn get_settings(state: tauri::State<'_, AppState>) -> Result<CombinedSettings, String> {
    info!("[get_settings]");
    let vad = read_lock(&state.settings).clone();
    let llm = read_lock(&state.llm_settings).clone();
    Ok(CombinedSettings {
        asr_language: vad.asr_language,
        auto_copy_mode: llm.auto_copy_mode,
    })
}

fn validate_language(s: &str) -> Result<(), String> {
    if !matches!(s, "" | "zh" | "en" | "ja" | "ko" | "yue") {
        return Err("asr_language must be one of '', zh, en, ja, ko, yue".to_string());
    }
    Ok(())
}

pub(crate) async fn apply_settings(
    new_settings: CombinedSettings,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!("[apply_settings]");
    validate_language(&new_settings.asr_language)?;

    let new_vad = VadSettings { asr_language: new_settings.asr_language };
    let new_llm = LlmSettings { auto_copy_mode: new_settings.auto_copy_mode };

    let settings_arc = Arc::clone(&state.settings);
    let llm_arc = Arc::clone(&state.llm_settings);
    let db_arc = Arc::clone(&state.db);

    *write_lock(&settings_arc) = new_vad.clone();
    *write_lock(&llm_arc) = new_llm.clone();

    let db = {
        let guard = mutex_lock(&db_arc);
        guard.as_ref().cloned().ok_or("Database not initialized")?
    };
    db.upsert_setting("asr.language".to_string(), new_vad.asr_language)
        .await
        .map_err(|e| e.to_string())?;
    db.upsert_setting(
        "llm.auto_copy_mode".to_string(),
        match new_llm.auto_copy_mode {
            AutoCopyMode::Off => "off",
            AutoCopyMode::English => "english",
            AutoCopyMode::OptimizedZh => "optimized_zh",
        }
        .to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) async fn load_vad_settings_from_db(db: &db::SpeechDatabase) -> VadSettings {
    let mut s = VadSettings::default();
    if let Ok(Some(v)) = db.get_setting("asr.language".to_string()).await {
        if matches!(v.as_str(), "" | "zh" | "en" | "ja" | "ko" | "yue") {
            s.asr_language = v;
        }
    }
    s
}

pub(crate) async fn load_llm_settings_from_db(db: &db::SpeechDatabase) -> LlmSettings {
    let mut s = LlmSettings::default();
    if let Ok(Some(v)) = db.get_setting("llm.auto_copy_mode".to_string()).await {
        s.auto_copy_mode = match v.as_str() {
            "off" => AutoCopyMode::Off,
            "optimized_zh" => AutoCopyMode::OptimizedZh,
            _ => AutoCopyMode::English,
        };
    }
    s
}
