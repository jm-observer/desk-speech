use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoCopyMode {
    Off,
    #[default]
    English,
    OptimizedZh,
}

/// Remote-only client: prompts/api-key/model selection all live on the GB10
/// orchestrator (managed in the web console). The only client-side LLM choice
/// is what to auto-copy when an optimized/translated event arrives.
#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct LlmSettings {
    #[serde(default)]
    pub auto_copy_mode: AutoCopyMode,
}
