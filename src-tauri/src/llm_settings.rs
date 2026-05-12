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
    pub discard_prompt_template: String,
    #[serde(default)]
    pub auto_copy_mode: AutoCopyMode,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            provider_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            selected_model: String::new(),
            optimize_prompt_template: "你是一个中文语音转写后处理助手，负责将语音识别得到的原始中文文本优化为更清晰、准确、适合程序开发、产品设计和工程沟通场景使用的中文表达。\n\n任务：\n1. 修正明显的错别字、同音误识别、断句错误和不自然表达。\n2. 去除不影响语义的口语噪音，例如“嗯”“啊”“呃”“这个”“就是”等。\n3. 补全必要标点，使句子更易读。\n4. 如果输入涉及代码、配置、命令、接口、路径、类名、函数名、变量名、产品名、模型名或缩写，必须保留这些技术标识，不要擅自改写。\n5. 如果输入本身是程序相关表述，请将其整理为更清晰的工程表达，但不要改变技术含义。\n6. 保持原意，不扩写，不编造，不添加输入中不存在的信息。\n\n输出要求：\n- 只返回一个 JSON 对象。\n- 不要返回 Markdown。\n- 不要添加解释、注释或前后缀文本。\n- JSON 必须包含字段 `text_optimized`。\n- `text_optimized` 的值必须是优化后的中文文本。\n\n输出格式：\n{\"text_optimized\":\"优化后的中文文本\"}"
                .to_string(),
            translate_prompt_template:
                "你是一个中译英翻译助手。输入是已优化的中文文本,请忠实翻译为英文,不添加解释或注释。返回 JSON:{\"text_english\":\"...\"}。"
                    .to_string(),
            discard_prompt_template: "你是一个语音转写质量判定助手。请判断以下文本段是否应该被丢弃(无意义/填充词/噪音).\n\n判定规则:\n1. 纯语气词/填充词(如嗯、啊、呃、ok、好的)→ 丢弃\n2. 仅单个称呼/姓名无实义(如张三、王老师)→ 丢弃\n3. 高重复低信息(同一 token 重复占比 >= 0.8,长度 <= 8)→ 丢弃\n4. 其他无实际语义的内容 → 丢弃\n5. 有明确语义、信息量充足的文本 → 保留\n\n返回 JSON:{\"decision\":\"KEEP\"|\"DISCARD\",\"confidence\":0.0-1.0,\"reason\":\"简短中文说明\"}\n\n文本信息:\n优化文本:{text_optimized}\n原始文本:{text_raw}\n英文翻译:{text_english}".to_string(),
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
