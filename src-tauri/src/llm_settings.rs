use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct LlmSettings {
    pub provider_url: String,
    pub api_key: String,
    pub selected_model: String,
    pub prompt_template: String,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            provider_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            selected_model: String::new(),
            prompt_template: "你是一个中英双语转写后处理助手。\n输入是语音识别文本，请返回 JSON：{\"text_optimized\":\"...\",\"text_english\":\"...\"}。"
                .to_string(),
        }
    }
}

pub fn validate_llm_settings(settings: &LlmSettings) -> Result<(), String> {
    if settings.provider_url.trim().is_empty() {
        return Err("provider_url cannot be empty".to_string());
    }
    if settings.prompt_template.trim().is_empty() {
        return Err("prompt_template cannot be empty".to_string());
    }
    Ok(())
}
