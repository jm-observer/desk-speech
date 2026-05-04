use std::future::Future;
use std::time::{Duration, Instant};

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs, ResponseFormat,
};
use async_openai::Client;
use log::{info, warn};
use serde::Deserialize;
use serde_json::Value;

use crate::llm_settings::LlmSettings;
use tokio::time::sleep;

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
}
