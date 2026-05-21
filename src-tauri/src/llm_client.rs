use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::llm_settings::LlmSettings;

const MODEL_CACHE_TTL: Duration = Duration::from_secs(300);

pub struct CachedModels {
    pub fetched_at: Instant,
    pub models: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModel>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenAIModel {
    id: String,
}

pub async fn list_models(settings: &LlmSettings) -> Result<Vec<String>, String> {
    let base_url = settings.provider_url.trim();
    let url = if base_url.ends_with("/v1") {
        format!("{}/models", base_url)
    } else if let Some(stripped) = base_url.strip_suffix('/') {
        format!("{}/v1/models", stripped)
    } else {
        format!("{}/v1/models", base_url)
    };

    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("failed to call model provider api: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("model provider api returned status: {}", resp.status()));
    }

    let models_resp: OpenAIModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("failed to deserialize api response: {}", e))?;

    let mut models = models_resp.data.into_iter().map(|m| m.id).collect::<Vec<_>>();
    models.sort();
    Ok(models)
}

pub fn model_cache_valid(cache: &CachedModels) -> bool {
    cache.fetched_at.elapsed() <= MODEL_CACHE_TTL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_cache_valid_respects_ttl_boundary() {
        let valid_cache = CachedModels {
            fetched_at: Instant::now() - Duration::from_secs(299),
            models: vec!["gpt-4o-mini".to_string()],
        };
        assert!(model_cache_valid(&valid_cache));

        let expired_cache = CachedModels {
            fetched_at: Instant::now() - Duration::from_secs(301),
            models: vec!["gpt-4o-mini".to_string()],
        };
        assert!(!model_cache_valid(&expired_cache));
    }
}
