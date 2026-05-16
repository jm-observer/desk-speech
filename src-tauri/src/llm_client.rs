use std::future::Future;
use std::time::{Duration, Instant};

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs, ReasoningEffort, ResponseFormat,
};
use async_openai::Client;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::quality_filter::QualityFilterConfig;
use crate::llm_settings::LlmSettings;
use tokio::time::sleep;

#[derive(Deserialize, Debug, Serialize)]
pub(crate) struct LlmJudgmentOutput {
    pub(crate) decision: String, // KEEP or DISCARD
    pub(crate) confidence: f32,
    pub(crate) reason: String,
}

#[derive(Debug)]
pub struct JudgmentInput {
    pub text_raw: String,
    pub text_optimized: Option<String>,
    pub text_english: Option<String>,
}

#[derive(Debug)]
pub struct JudgmentResult {
    pub decision: String, // KEEP or DISCARD
    pub confidence: f32,
    pub reason: String,
}

const MODEL_CACHE_TTL: Duration = Duration::from_secs(300);
const LLM_RETRY_MAX_ATTEMPTS: u32 = 3;
const LLM_RETRY_DELAY: Duration = Duration::from_millis(500);

async fn retry_task<T, F, Fut>(task_name: &str, max_attempts: u32, delay: Duration, mut task: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let attempts = std::cmp::max(max_attempts, 1);
    let mut last_error = String::new();

    for attempt in 1..=attempts {
        match task().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = e;
                if attempt < attempts {
                    warn!(
                        "[{}] attempt {}/{} failed: {}. Retrying in {:?}...",
                        task_name, attempt, attempts, last_error, delay
                    );
                    sleep(delay).await;
                }
            }
        }
    }

    Err(last_error)
}

pub struct CachedModels {
    pub fetched_at: Instant,
    pub models: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct LlmOptimizeOutput {
    pub(crate) text_optimized: String,
}

#[derive(Deserialize, Debug)]
struct LlmTranslateOutput {
    text_english: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModel>,
    #[allow(dead_code)]
    object: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenAIModel {
    id: String,
    #[allow(dead_code)]
    created: Option<u64>,
    #[serde(rename = "object")]
    #[allow(dead_code)]
    object_type: Option<String>,
    #[allow(dead_code)]
    owned_by: Option<String>,
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

pub async fn optimize_text(settings: &LlmSettings, input_text: &str) -> Result<String, String> {
    retry_task("optimize_text", LLM_RETRY_MAX_ATTEMPTS, LLM_RETRY_DELAY, || async {
        let content = chat_json_completion(settings, &settings.optimize_prompt_template, input_text).await?;
        let json = extract_json(&content)?;
        let parsed: LlmOptimizeOutput = serde_json::from_value(json).map_err(|e| e.to_string())?;
        Ok(parsed.text_optimized)
    })
    .await
    .map(|text| {
        info!("optimize output: {text}");
        text
    })
}

pub async fn translate_text(settings: &LlmSettings, optimized_text: &str) -> Result<String, String> {
    retry_task("translate_text", LLM_RETRY_MAX_ATTEMPTS, LLM_RETRY_DELAY, || async {
        let content = chat_json_completion(settings, &settings.translate_prompt_template, optimized_text).await?;
        let json = extract_json(&content)?;
        let parsed: LlmTranslateOutput = serde_json::from_value(json).map_err(|e| e.to_string())?;
        Ok(parsed.text_english)
    })
    .await
    .map(|text| {
        info!("translate output: {text}");
        text
    })
}

async fn chat_json_completion(settings: &LlmSettings, system_prompt: &str, input_text: &str) -> Result<String, String> {
    let client = build_client(settings);
    let system_message = ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
        content: system_prompt.to_string().into(),
        name: None,
    });
    let user_message = ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: input_text.to_string().into(),
        name: None,
    });

    let request = CreateChatCompletionRequestArgs::default()
        .model(settings.selected_model.clone())
        .messages(vec![system_message, user_message])
        .reasoning_effort(ReasoningEffort::None)
        .response_format(ResponseFormat::Text)
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.chat().create(request).await.map_err(|e| e.to_string())?;
    response
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .cloned()
        .ok_or("empty llm response".to_string())
}

fn build_client(settings: &LlmSettings) -> Client<OpenAIConfig> {
    let config = OpenAIConfig::new()
        .with_api_key(settings.api_key.clone())
        .with_api_base(settings.provider_url.clone());
    Client::with_config(config)
}

pub(crate) fn extract_json(content: &str) -> Result<Value, String> {
    serde_json::from_str(content).or_else(|_| {
        let start = content.find('{').ok_or("missing JSON object start")?;
        let end = content.rfind('}').ok_or("missing JSON object end")?;
        serde_json::from_str(&content[start..=end]).map_err(|e| e.to_string())
    })
}

// ---------------------------------------------------------------------------
// Rule-based discard (lightweight, no LLM call)
// ---------------------------------------------------------------------------

/// Check if text has high repetition of the same token.
fn is_high_repetition(text: &str, config: &QualityFilterConfig) -> bool {
    let t = text.trim();
    let len = t.chars().count();
    if len == 0 || len > 8 {
        return false;
    }
    // Count most frequent character.
    let mut freq: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    for c in t.chars() {
        *freq.entry(c).or_insert(0) += 1;
    }
    if let Some(&max_count) = freq.values().max() {
        // Same token repetition >= threshold
        return (max_count as f32 / len as f32) >= config.repeat_ratio_threshold;
    }
    false
}

/// Run lightweight rules. Returns `Some(true)` if text should be discarded by rule.
pub fn check_discard_rules(text: &str, config: &QualityFilterConfig) -> bool {
    if !config.enabled {
        return false;
    }
    let normalized = text.trim();
    if normalized.is_empty() {
        return true;
    }
    // Rule 1: Very short ASCII tokens (e.g. ok/hi/a) are low information.
    if normalized.chars().count() < 3 && normalized.chars().all(|c| c.is_ascii_alphanumeric()) {
        return true;
    }
    // Rule 2: High repetition
    if is_high_repetition(normalized, config) {
        return true;
    }
    false
}

/// Truncate and strip control characters from text before embedding in prompts.
fn sanitize_for_prompt(text: &str) -> String {
    const MAX_PROMPT_INPUT_LEN: usize = 2000;
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(MAX_PROMPT_INPUT_LEN)
        .collect()
}

/// Call LLM to perform judgment. Returns (decision, confidence, reason).
/// On failure, returns ("FAILED", 0.0, "llm_error").
pub async fn judge_discard(
    settings: &LlmSettings,
    config: &QualityFilterConfig,
    input: &JudgmentInput,
) -> Result<JudgmentResult, String> {
    // Sanitize inputs to mitigate prompt injection from ASR output
    let sanitized_raw = sanitize_for_prompt(&input.text_raw);
    let sanitized_optimized = input.text_optimized.as_deref().map(sanitize_for_prompt);
    let sanitized_english = input.text_english.as_deref().map(sanitize_for_prompt);

    // Use config prompt template if available, fallback to settings
    let system_prompt = if !config.llm_prompt_template.is_empty() && config.has_placeholders() {
        config.render_prompt_template(
            &sanitized_raw,
            sanitized_optimized.as_deref(),
            sanitized_english.as_deref(),
        )
    } else {
        settings
            .discard_prompt_template
            .replace("{text_optimized}", sanitized_optimized.as_deref().unwrap_or(""))
            .replace("{text_raw}", &sanitized_raw)
            .replace("{text_english}", sanitized_english.as_deref().unwrap_or(""))
    };

    // Primary input: text_optimized > text_raw
    let user_input = input
        .text_optimized
        .as_ref()
        .or(Some(&input.text_raw))
        .cloned()
        .unwrap_or_default();

    let content = chat_json_completion(settings, &system_prompt, &user_input).await?;

    let json_value = extract_json(&content)?;
    let parsed: LlmJudgmentOutput = serde_json::from_value(json_value).map_err(|e| e.to_string())?;

    // Apply confidence threshold from config
    let threshold = config.discard_confidence_threshold;
    let final_decision = if parsed.decision == "DISCARD" && parsed.confidence >= threshold {
        "DISCARD".to_string()
    } else if parsed.decision == "DISCARD" {
        // Low confidence DISCARD → conservative KEEP
        "KEEP".to_string()
    } else {
        "KEEP".to_string()
    };

    Ok(JudgmentResult {
        decision: final_decision,
        confidence: parsed.confidence,
        reason: parsed.reason,
    })
}

/// Evaluate the final judgment result.
/// Returns true if the segment should be discarded.
/// Applies confidence threshold: DISCARD with confidence < threshold is kept.
pub fn evaluate_judgment(result: &JudgmentResult, config: &QualityFilterConfig) -> bool {
    if result.decision == "KEEP" {
        return false;
    }
    // DISCARD decision
    let threshold = config.discard_confidence_threshold;
    if result.confidence >= threshold {
        return true;
    }
    // Low confidence DISCARD → conservative KEEP
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_task_success_first_time() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(retry_task("test", 3, Duration::from_millis(1), || async {
            Ok::<&str, String>("success")
        }));
        assert_eq!(result.unwrap(), "success");
    }

    #[test]
    fn retry_task_success_after_retries() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut count = 0;
        let result = rt.block_on(retry_task("test", 3, Duration::from_millis(1), || {
            count += 1;
            async move {
                if count < 3 {
                    Err("fail".to_string())
                } else {
                    Ok("recovered")
                }
            }
        }));
        assert_eq!(result.unwrap(), "recovered");
        assert_eq!(count, 3);
    }

    #[test]
    fn retry_task_exhaust_attempts() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut count = 0;
        let result = rt.block_on(retry_task("test", 3, Duration::from_millis(1), || {
            count += 1;
            async move { Err::<&str, String>(format!("error {}", count)) }
        }));
        assert_eq!(result.unwrap_err(), "error 3");
        assert_eq!(count, 3);
    }

    #[test]
    fn retry_task_zero_attempts_handles_gracefully() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut count = 0;
        let result = rt.block_on(retry_task("test", 0, Duration::from_millis(1), || {
            count += 1;
            async move { Ok::<&str, String>("one_shot") }
        }));
        assert_eq!(result.unwrap(), "one_shot");
        assert_eq!(count, 1);
    }

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

    // ---------------------------------------------------------------------------
    // Rule-based discard tests
    // ---------------------------------------------------------------------------

    #[test]
    fn check_discard_rules_short_text() {
        let config = QualityFilterConfig::default();
        assert!(check_discard_rules("ok", &config));
        assert!(check_discard_rules("嗯", &config));
        assert!(check_discard_rules("a", &config));
    }

    #[test]
    fn check_discard_rules_repeated_short_tokens() {
        let config = QualityFilterConfig::default();
        assert!(check_discard_rules("嗯嗯", &config));
        assert!(check_discard_rules("啊啊啊啊", &config));
        assert!(check_discard_rules("对对", &config));
    }

    #[test]
    fn check_discard_rules_high_repetition() {
        let config = QualityFilterConfig::default();
        assert!(check_discard_rules("啊啊啊啊", &config));
        assert!(check_discard_rules("嗯嗯嗯", &config));
    }

    #[test]
    fn check_discard_rules_keeps_meaningful_text() {
        let config = QualityFilterConfig::default();
        assert!(!check_discard_rules("今天天气不错", &config));
        assert!(!check_discard_rules("你好，请问有什么可以帮助你的", &config));
        assert!(!check_discard_rules("会议开始", &config));
    }
}
