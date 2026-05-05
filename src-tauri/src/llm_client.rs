use std::future::Future;
use std::time::{Duration, Instant};

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs, ResponseFormat,
};
use async_openai::Client;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm_settings::LlmSettings;
use tokio::time::sleep;

#[derive(Deserialize, Debug, Serialize)]
struct LlmJudgmentOutput {
    decision: String, // KEEP or DISCARD
    confidence: f32,
    reason: String,
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

const JUDGMENT_CONFIDENCE_THRESHOLD: f32 = 0.65;

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
struct LlmOptimizeOutput {
    text_optimized: String,
}

#[derive(Deserialize, Debug)]
struct LlmTranslateOutput {
    text_english: String,
}

pub async fn list_models(settings: &LlmSettings) -> Result<Vec<String>, String> {
    let client = build_client(settings);
    let resp = client.models().list().await.map_err(|e| e.to_string())?;
    let mut models = resp.data.into_iter().map(|m| m.id).collect::<Vec<_>>();
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
        .response_format(ResponseFormat::JsonObject)
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

fn extract_json(content: &str) -> Result<Value, String> {
    serde_json::from_str(content).or_else(|_| {
        let start = content.find('{').ok_or("missing JSON object start")?;
        let end = content.rfind('}').ok_or("missing JSON object end")?;
        serde_json::from_str(&content[start..=end]).map_err(|e| e.to_string())
    })
}

// ---------------------------------------------------------------------------
// Rule-based discard (lightweight, no LLM call)
// ---------------------------------------------------------------------------

/// Check if text is composed only of filler words / interjections.
fn is_pure_filler(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    const FILLERS: &[&str] = &[
        "ok", "okay", "嗯", "啊", "呃", "嗯嗯", "嗯嗯", "哦", "哎", "唉",
        "对", "对对", "是", "是的", "好", "好好", "嗯哼", "嘛",
    ];
    FILLERS.contains(&t.as_str())
}

/// Check if text matches a single name/title pattern (no verbs/real meaning).
fn is_single_name(text: &str) -> bool {
    let t = text.trim();
    // Heuristic: 2-4 Chinese characters, no verbs commonly found in speech.
    let char_count = t.chars().count();
    if !(2..=4).contains(&char_count) {
        return false;
    }
    // Check if all chars are CJK unified ideographs (simplified name check).
    let all_cjk = t.chars().all(|c| {
        ('\u{4e00}'..='\u{9fff}').contains(&c)
            || ('\u{3400}'..='\u{4dbf}').contains(&c)
    });
    // Exclude common non-name words.
    let non_names = ["老师", "同学", "朋友", "大家", "我们", "你们", "他们", "这个", "那个"];
    if non_names.iter().any(|n| t.contains(*n)) {
        return false;
    }
    all_cjk
}

/// Check if text has high repetition of the same token.
fn is_high_repetition(text: &str) -> bool {
    let t = text.trim();
    let len = t.len();
    if len == 0 || len > 8 {
        return false;
    }
    // Count most frequent character.
    let mut freq: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    for c in t.chars() {
        *freq.entry(c).or_insert(0) += 1;
    }
    if let Some(&max_count) = freq.values().max() {
        // Same token repetition >= 0.8
        return (max_count as f32 / len as f32) >= 0.8;
    }
    false
}

/// Run lightweight rules. Returns `Some(true)` if text should be discarded by rule.
pub fn check_discard_rules(text: &str) -> bool {
    let normalized = text.trim();
    if normalized.is_empty() {
        return true;
    }
    // Rule 1: Character length < 3
    if normalized.chars().count() < 3 {
        return true;
    }
    // Rule 2: Pure filler
    if is_pure_filler(normalized) {
        return true;
    }
    // Rule 3: Single name/title
    if is_single_name(normalized) {
        return true;
    }
    // Rule 4: High repetition
    if is_high_repetition(normalized) {
        return true;
    }
    false
}

/// Call LLM to perform judgment. Returns (decision, confidence, reason).
/// On failure, returns ("FAILED", 0.0, "llm_error").
pub async fn judge_discard(settings: &LlmSettings, input: &JudgmentInput) -> Result<JudgmentResult, String> {
    let system_prompt = settings.discard_prompt_template
        .replace("{text_optimized}", input.text_optimized.as_deref().unwrap_or(""))
        .replace("{text_raw}", &input.text_raw)
        .replace("{text_english}", input.text_english.as_deref().unwrap_or(""));

    // Primary input: text_optimized > text_raw
    let user_input = input.text_optimized
        .as_ref()
        .or(Some(&input.text_raw))
        .cloned()
        .unwrap_or_default();

    let content = chat_json_completion(settings, &system_prompt, &user_input).await?;

    let json_value = extract_json(&content)?;
    let parsed: LlmJudgmentOutput = serde_json::from_value(json_value).map_err(|e| e.to_string())?;

    // Apply confidence threshold
    let final_decision = if parsed.decision == "DISCARD" && parsed.confidence >= JUDGMENT_CONFIDENCE_THRESHOLD {
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
/// Applies confidence threshold: DISCARD with confidence < 0.65 is kept.
pub fn evaluate_judgment(result: &JudgmentResult) -> bool {
    if result.decision == "KEEP" {
        return false;
    }
    // DISCARD decision
    if result.confidence >= JUDGMENT_CONFIDENCE_THRESHOLD {
        return true;
    }
    // Low confidence DISCARD → conservative KEEP
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_accepts_plain_json() {
        let content = r#"{"text_optimized":"你好","text_english":"hello"}"#;
        let value = extract_json(content).unwrap();
        assert_eq!(value["text_optimized"], "你好");
        assert_eq!(value["text_english"], "hello");
    }

    #[test]
    fn extract_json_accepts_wrapped_json() {
        let content = r#"结果如下：
{"text_optimized":"测试","text_english":"test"}
谢谢"#;
        let value = extract_json(content).unwrap();
        assert_eq!(value["text_optimized"], "测试");
        assert_eq!(value["text_english"], "test");
    }

    #[test]
    fn extract_json_rejects_invalid_payload() {
        let content = "no json here";
        let err = extract_json(content).unwrap_err();
        assert!(err.contains("missing JSON object start"));
    }

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
        assert!(check_discard_rules("ok"));
        assert!(check_discard_rules("嗯"));
        assert!(check_discard_rules("a"));
    }

    #[test]
    fn check_discard_rules_filler_words() {
        assert!(check_discard_rules("嗯嗯"));
        assert!(check_discard_rules("啊"));
        assert!(check_discard_rules("对对"));
    }

    #[test]
    fn check_discard_rules_single_name() {
        assert!(check_discard_rules("张三"));
        assert!(check_discard_rules("李明"));
    }

    #[test]
    fn check_discard_rules_high_repetition() {
        assert!(check_discard_rules("啊啊啊啊"));
        assert!(check_discard_rules("嗯嗯嗯"));
    }

    #[test]
    fn check_discard_rules_keeps_meaningful_text() {
        assert!(!check_discard_rules("今天天气不错"));
        assert!(!check_discard_rules("你好，请问有什么可以帮助你的"));
        assert!(!check_discard_rules("会议开始"));
    }

    #[test]
    fn check_discard_rules_excludes_common_non_names() {
        assert!(!check_discard_rules("老师"));
        assert!(!check_discard_rules("同学"));
        assert!(!check_discard_rules("大家"));
    }

    #[test]
    fn evaluate_judgment_keeps_discard_when_confidence_high() {
        let result = JudgmentResult {
            decision: "DISCARD".to_string(),
            confidence: 0.8,
            reason: "filler".to_string(),
        };
        assert!(evaluate_judgment(&result));
    }

    #[test]
    fn evaluate_judgment_Keeps_discard_when_confidence_low() {
        let result = JudgmentResult {
            decision: "DISCARD".to_string(),
            confidence: 0.5,
            reason: "uncertain".to_string(),
        };
        assert!(!evaluate_judgment(&result));
    }

    #[test]
    fn evaluate_judgment_keeps_keep_decision() {
        let result = JudgmentResult {
            decision: "KEEP".to_string(),
            confidence: 0.9,
            reason: "meaningful".to_string(),
        };
        assert!(!evaluate_judgment(&result));
    }
}
