use std::sync::Arc;

use log::info;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::llm_settings::{AutoCopyMode, LlmSettings, MAX_MERGE_WINDOW_MS};
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

/// Built-in default orchestrator URL — surfaced as the first item in the
/// connection dropdown and used on a brand-new install.
pub(crate) const DEFAULT_REMOTE_URL: &str = "ws://192.168.0.68:8090/stream";

/// Combined settings DTO exchanged with the frontend — the choices the
/// desktop client makes: which ASR language to request, whether to auto-copy
/// the optimized Chinese / English translation to the clipboard, how long
/// the short-gap auto-copy stitch window stays open (ms; 0 disables merging),
/// and which remote orchestrator URL to connect to (`remote_url` + user-
/// managed `remote_url_presets`).
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CombinedSettings {
    asr_language: String,
    auto_copy_mode: AutoCopyMode,
    merge_window_ms: u64,
    remote_url: String,
    remote_url_presets: Vec<String>,
    #[serde(default)]
    want_secondary: bool,
}

pub(crate) fn get_settings(state: tauri::State<'_, AppState>) -> Result<CombinedSettings, String> {
    info!("[get_settings]");
    let vad = read_lock(&state.settings).clone();
    let llm = read_lock(&state.llm_settings).clone();
    let url = read_lock(&state.remote_url).clone();
    let presets = read_lock(&state.remote_url_presets).clone();
    Ok(CombinedSettings {
        asr_language: vad.asr_language,
        auto_copy_mode: llm.auto_copy_mode,
        merge_window_ms: llm.merge_window_ms,
        remote_url: url,
        remote_url_presets: presets,
        want_secondary: llm.want_secondary,
    })
}

fn validate_language(s: &str) -> Result<(), String> {
    if !matches!(s, "" | "zh" | "en" | "ja" | "ko" | "yue") {
        return Err("asr_language must be one of '', zh, en, ja, ko, yue".to_string());
    }
    Ok(())
}

fn validate_url(s: &str) -> Result<(), String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("remote_url 不能为空".to_string());
    }
    if !(trimmed.starts_with("ws://") || trimmed.starts_with("wss://")) {
        return Err("remote_url 必须以 ws:// 或 wss:// 开头".to_string());
    }
    Ok(())
}

pub(crate) async fn apply_settings(
    new_settings: CombinedSettings,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!("[apply_settings]");
    validate_language(&new_settings.asr_language)?;
    validate_url(&new_settings.remote_url)?;

    // Normalise + de-dupe presets, drop blanks and the built-in default
    // (the built-in is always shown anyway).
    let mut cleaned_presets: Vec<String> = Vec::new();
    for p in &new_settings.remote_url_presets {
        let t = p.trim();
        if t.is_empty() || t == DEFAULT_REMOTE_URL {
            continue;
        }
        // skip malformed entries silently rather than refusing the whole save
        if !(t.starts_with("ws://") || t.starts_with("wss://")) {
            continue;
        }
        let s = t.to_string();
        if !cleaned_presets.contains(&s) {
            cleaned_presets.push(s);
        }
    }

    let new_vad = VadSettings { asr_language: new_settings.asr_language };
    let new_llm = LlmSettings {
        auto_copy_mode: new_settings.auto_copy_mode,
        merge_window_ms: new_settings.merge_window_ms.min(MAX_MERGE_WINDOW_MS),
        want_secondary: new_settings.want_secondary,
    };
    let new_url = new_settings.remote_url.trim().to_string();

    let settings_arc = Arc::clone(&state.settings);
    let llm_arc = Arc::clone(&state.llm_settings);
    let url_arc = Arc::clone(&state.remote_url);
    let presets_arc = Arc::clone(&state.remote_url_presets);
    let db_arc = Arc::clone(&state.db);

    *write_lock(&settings_arc) = new_vad.clone();
    *write_lock(&llm_arc) = new_llm.clone();
    *write_lock(&url_arc) = new_url.clone();
    *write_lock(&presets_arc) = cleaned_presets.clone();

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
    db.upsert_setting(
        "llm.merge_window_ms".to_string(),
        new_llm.merge_window_ms.to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;
    db.upsert_setting(
        "ui.want_secondary".to_string(),
        if new_llm.want_secondary { "1".into() } else { "0".into() },
    )
    .await
    .map_err(|e| e.to_string())?;
    db.upsert_setting("remote.url".to_string(), new_url)
        .await
        .map_err(|e| e.to_string())?;
    let presets_json = serde_json::to_string(&cleaned_presets).map_err(|e| e.to_string())?;
    db.upsert_setting("remote.url_presets".to_string(), presets_json)
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
    if let Ok(Some(v)) = db.get_setting("llm.merge_window_ms".to_string()).await {
        if let Ok(n) = v.parse::<u64>() {
            s.merge_window_ms = n.min(MAX_MERGE_WINDOW_MS);
        }
    }
    if let Ok(Some(v)) = db.get_setting("ui.want_secondary".to_string()).await {
        s.want_secondary = !matches!(v.as_str(), "0" | "off" | "false" | "");
    }
    s
}

/// Load `(remote.url, remote.url_presets)` from the local DB, falling back
/// to the built-in default on a fresh install. Invalid persisted values are
/// silently replaced with the default so a corrupt row can't brick startup.
pub(crate) async fn load_remote_settings_from_db(
    db: &db::SpeechDatabase,
) -> (String, Vec<String>) {
    let url = match db.get_setting("remote.url".to_string()).await {
        Ok(Some(v))
            if !v.trim().is_empty() && (v.starts_with("ws://") || v.starts_with("wss://")) =>
        {
            v.trim().to_string()
        }
        _ => DEFAULT_REMOTE_URL.to_string(),
    };
    let presets = match db.get_setting("remote.url_presets".to_string()).await {
        Ok(Some(v)) => serde_json::from_str::<Vec<String>>(&v)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| {
                let t = s.trim();
                !t.is_empty()
                    && (t.starts_with("ws://") || t.starts_with("wss://"))
                    && t != DEFAULT_REMOTE_URL
            })
            .collect(),
        _ => Vec::new(),
    };
    (url, presets)
}
