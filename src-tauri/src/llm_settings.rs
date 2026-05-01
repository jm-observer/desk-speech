use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoCopyMode {
    Off,
    #[default]
    English,
    OptimizedZh,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct LlmSettings {
    pub provider_url: String,
    pub api_key: String,
    pub selected_model: String,
    pub optimize_prompt_template: String,
    pub translate_prompt_template: String,
    #[serde(default)]
    pub auto_copy_mode: AutoCopyMode,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            provider_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            selected_model: String::new(),
            optimize_prompt_template:
                "你是一个中文转写后处理助手。输入是语音识别文本，请修正错别字、去除口语噪音（如“嗯”“啊”等）、补全标点，保持原意不扩写。返回 JSON：{\"text_optimized\":\"...\"}。"
                    .to_string(),
            translate_prompt_template:
                "你是一个中译英翻译助手。输入是已优化的中文文本，请忠实翻译为英文，不添加解释或注释。返回 JSON：{\"text_english\":\"...\"}。"
                    .to_string(),
            auto_copy_mode: AutoCopyMode::default(),
        }
    }
}

pub fn validate_llm_settings(settings: &LlmSettings) -> Result<(), String> {
    if settings.provider_url.trim().is_empty() {
        return Err("provider_url cannot be empty".to_string());
    }
    if settings.optimize_prompt_template.trim().is_empty() {
        return Err("optimize_prompt_template cannot be empty".to_string());
    }
    if settings.translate_prompt_template.trim().is_empty() {
        return Err("translate_prompt_template cannot be empty".to_string());
    }
    Ok(())
}
