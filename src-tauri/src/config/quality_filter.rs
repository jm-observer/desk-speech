use serde::{Deserialize, Serialize};

/// Quality filter configuration for finalization discard judgment.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QualityFilterConfig {
    /// LLM prompt template for judgment.
    /// Supports placeholders: {{text_raw}}, {{text_optimized}}, {{text_english}}
    pub llm_prompt_template: String,

    /// Confidence threshold for DISCARD decision (0.0..=1.0).
    /// DISCARD with confidence >= threshold → discard.
    /// DISCARD with confidence < threshold → keep (conservative).
    #[serde(default = "default_discard_confidence_threshold")]
    pub discard_confidence_threshold: f32,

    /// Silence window in milliseconds before triggering finalization check.
    #[serde(default = "default_silence_window_ms")]
    pub silence_window_ms: u64,

    /// Ratio threshold for high repetition detection (0.0..=1.0).
    #[serde(default = "default_repeat_ratio_threshold")]
    pub repeat_ratio_threshold: f32,

    /// Total switch for quality filter. When false, skip discard logic.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Schema version for migration compatibility.
    #[serde(default = "default_version")]
    pub version: u32,
}

// ─── Default values ─────────────────────────────────────────────────────────

fn default_discard_confidence_threshold() -> f32 {
    0.65
}

fn default_silence_window_ms() -> u64 {
    10_000
}

fn default_repeat_ratio_threshold() -> f32 {
    0.8
}

fn default_enabled() -> bool {
    true
}

fn default_version() -> u32 {
    1
}

impl Default for QualityFilterConfig {
    fn default() -> Self {
        Self {
            llm_prompt_template: String::new(), // Will be populated from LlmSettings or built-in default
            discard_confidence_threshold: default_discard_confidence_threshold(),
            silence_window_ms: default_silence_window_ms(),
            repeat_ratio_threshold: default_repeat_ratio_threshold(),
            enabled: default_enabled(),
            version: default_version(),
        }
    }
}

// ─── Validation ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigValidationError {
    pub field: String,
    pub message: String,
}

impl QualityFilterConfig {
    /// Validate configuration values. Returns errors if any field is invalid.
    pub fn validate(&self) -> Result<(), Vec<ConfigValidationError>> {
        let mut errors = Vec::new();

        // Validate discard_confidence_threshold range
        if !(0.0..=1.0).contains(&self.discard_confidence_threshold) {
            errors.push(ConfigValidationError {
                field: "discard_confidence_threshold".to_string(),
                message: format!("Must be between 0.0 and 1.0, got {}", self.discard_confidence_threshold),
            });
        }

        // Validate silence_window_ms minimum
        if self.silence_window_ms < 1000 {
            errors.push(ConfigValidationError {
                field: "silence_window_ms".to_string(),
                message: format!("Must be >= 1000ms, got {}ms", self.silence_window_ms),
            });
        }

        // Validate repeat_ratio_threshold range
        if !(0.0..=1.0).contains(&self.repeat_ratio_threshold) {
            errors.push(ConfigValidationError {
                field: "repeat_ratio_threshold".to_string(),
                message: format!("Must be between 0.0 and 1.0, got {}", self.repeat_ratio_threshold),
            });
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Render the prompt template with given values.
    pub fn render_prompt_template(
        &self,
        text_raw: &str,
        text_optimized: Option<&str>,
        text_english: Option<&str>,
    ) -> String {
        let mut template = self.llm_prompt_template.clone();
        template = template.replace("{{text_raw}}", text_raw);
        template = template.replace("{{text_optimized}}", text_optimized.unwrap_or(""));
        template = template.replace("{{text_english}}", text_english.unwrap_or(""));
        template
    }

    /// Check if the template has any placeholders.
    pub fn has_placeholders(&self) -> bool {
        self.llm_prompt_template.contains("{{text_raw}}")
            || self.llm_prompt_template.contains("{{text_optimized}}")
            || self.llm_prompt_template.contains("{{text_english}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = QualityFilterConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn threshold_out_of_range_fails_validation() {
        let config = QualityFilterConfig {
            discard_confidence_threshold: 1.5,
            ..QualityFilterConfig::default()
        };
        let errors = config.validate().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "discard_confidence_threshold");
    }

    #[test]
    fn silence_window_too_small_fails_validation() {
        let config = QualityFilterConfig {
            silence_window_ms: 500,
            ..QualityFilterConfig::default()
        };
        let errors = config.validate().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "silence_window_ms");
    }

    #[test]
    fn empty_prompt_template_is_allowed() {
        let config = QualityFilterConfig {
            llm_prompt_template: String::new(),
            ..QualityFilterConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn render_prompt_template_replaces_placeholders() {
        let config = QualityFilterConfig {
            llm_prompt_template: "Raw: {{text_raw}}, Opt: {{text_optimized}}, En: {{text_english}}".to_string(),
            ..QualityFilterConfig::default()
        };
        let rendered = config.render_prompt_template("hello", Some("hello"), Some("你好"));
        assert_eq!(rendered, "Raw: hello, Opt: hello, En: 你好");
    }

    #[test]
    fn render_prompt_template_handles_none_values() {
        let config = QualityFilterConfig {
            llm_prompt_template: "{{text_raw}} {{text_optimized}} {{text_english}}".to_string(),
            ..QualityFilterConfig::default()
        };
        let rendered = config.render_prompt_template("hello", None, None);
        assert_eq!(rendered, "hello  ");
    }

    #[test]
    fn has_placeholders_returns_true() {
        let config = QualityFilterConfig {
            llm_prompt_template: "{{text_raw}}".to_string(),
            ..QualityFilterConfig::default()
        };
        assert!(config.has_placeholders());
    }

    #[test]
    fn has_placeholders_returns_false_when_empty() {
        let config = QualityFilterConfig::default();
        assert!(!config.has_placeholders());
    }
}
