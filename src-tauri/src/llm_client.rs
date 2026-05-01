use std::time::{Duration, Instant};

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs, ResponseFormat,
};
use async_openai::Client;
use log::info;
use serde::Deserialize;
use serde_json::Value;

use crate::llm_settings::LlmSettings;

const MODEL_CACHE_TTL: Duration = Duration::from_secs(300);

pub struct CachedModels {
    pub fetched_at: Instant,
    pub models: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct LlmPostprocessOutput {
    text_optimized: String,
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

pub async fn postprocess_text(settings: &LlmSettings, input_text: &str) -> Result<(String, String), String> {
    let client = build_client(settings);
    let system_message = ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
        content: settings.prompt_template.clone().into(),
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
    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .ok_or("empty llm response")?;

    let json = extract_json(content)?;
    let parsed: LlmPostprocessOutput = serde_json::from_value(json).map_err(|e| e.to_string())?;
    info!("{parsed:?}");
    // if parsed.text_optimized.trim().is_empty() || parsed.text_english.trim().is_empty() {
    //     return Err("llm response contains empty fields".to_string());
    // }
    Ok((parsed.text_optimized, parsed.text_english))
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
