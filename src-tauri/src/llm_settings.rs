use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoCopyMode {
    Off,
    #[default]
    English,
    OptimizedZh,
}

/// Default auto-copy "stitch window" (ms): consecutive short-gap segments
/// within this window are concatenated into one clipboard paste. User-
/// configurable from the desktop client; see `remote.rs::next_clipboard_text`.
pub const DEFAULT_MERGE_WINDOW_MS: u64 = 3000;

/// Upper bound for the configurable stitch window (ms).
pub const MAX_MERGE_WINDOW_MS: u64 = 60_000;

fn default_merge_window_ms() -> u64 {
    DEFAULT_MERGE_WINDOW_MS
}

/// Remote-only client: prompts/api-key/model selection all live on the GB10
/// orchestrator (managed in the web console). The client-side LLM choices are
/// what to auto-copy when an optimized/translated event arrives, and how long
/// the short-gap stitch window stays open.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct LlmSettings {
    #[serde(default)]
    pub auto_copy_mode: AutoCopyMode,
    /// Auto-copy stitch window in milliseconds. `0` disables merging.
    #[serde(default = "default_merge_window_ms")]
    pub merge_window_ms: u64,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            auto_copy_mode: AutoCopyMode::default(),
            merge_window_ms: DEFAULT_MERGE_WINDOW_MS,
        }
    }
}
