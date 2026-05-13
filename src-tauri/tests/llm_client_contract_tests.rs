#[path = "../src/config/mod.rs"]
mod config;
#[path = "../src/llm_client.rs"]
mod llm_client;
#[path = "../src/llm_settings.rs"]
mod llm_settings;

use config::quality_filter::QualityFilterConfig;
use llm_client::{evaluate_judgment, extract_json, JudgmentResult, LlmJudgmentOutput, LlmOptimizeOutput};

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
fn optimize_output_parses_required_field() {
    let json = serde_json::json!({"text_optimized": "优化后的文本"});
    let parsed: LlmOptimizeOutput = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.text_optimized, "优化后的文本");
}

#[test]
fn optimize_output_allows_extra_fields() {
    let json = serde_json::json!({
        "text_optimized": "保留技术标识",
        "extra": "ignored"
    });
    let parsed: LlmOptimizeOutput = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.text_optimized, "保留技术标识");
}

#[test]
fn optimize_output_rejects_missing_required_field() {
    let json = serde_json::json!({"text": "优化后的文本"});
    let err = serde_json::from_value::<LlmOptimizeOutput>(json).unwrap_err();
    assert!(err.to_string().contains("text_optimized"));
}

#[test]
fn optimize_output_rejects_non_string_required_field() {
    let json = serde_json::json!({"text_optimized": 123});
    let err = serde_json::from_value::<LlmOptimizeOutput>(json).unwrap_err();
    assert!(err.to_string().contains("string"));
}

#[test]
fn judgment_output_parses_required_fields() {
    let json = serde_json::json!({
        "decision": "KEEP",
        "confidence": 0.9,
        "reason": "包含明确语义，应保留"
    });
    let parsed: LlmJudgmentOutput = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.decision, "KEEP");
    assert_eq!(parsed.confidence, 0.9);
    assert_eq!(parsed.reason, "包含明确语义，应保留");
}

#[test]
fn judgment_output_allows_extra_fields() {
    let json = serde_json::json!({
        "decision": "DISCARD",
        "confidence": 0.95,
        "reason": "明显噪音",
        "extra": "ignored"
    });
    let parsed: LlmJudgmentOutput = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.decision, "DISCARD");
    assert_eq!(parsed.confidence, 0.95);
    assert_eq!(parsed.reason, "明显噪音");
}

#[test]
fn judgment_output_rejects_missing_required_field() {
    let json = serde_json::json!({
        "decision": "KEEP",
        "confidence": 0.9
    });
    let err = serde_json::from_value::<LlmJudgmentOutput>(json).unwrap_err();
    assert!(err.to_string().contains("reason"));
}

#[test]
fn judgment_output_rejects_non_numeric_confidence() {
    let json = serde_json::json!({
        "decision": "KEEP",
        "confidence": "high",
        "reason": "包含明确语义，应保留"
    });
    let err = serde_json::from_value::<LlmJudgmentOutput>(json).unwrap_err();
    assert!(err.to_string().contains("f32"));
}

#[test]
fn evaluate_judgment_keeps_discard_when_confidence_high() {
    let config = QualityFilterConfig::default();
    let result = JudgmentResult {
        decision: "DISCARD".to_string(),
        confidence: 0.8,
        reason: "filler".to_string(),
    };
    assert!(evaluate_judgment(&result, &config));
}

#[test]
fn evaluate_judgment_keeps_discard_when_confidence_low() {
    let config = QualityFilterConfig::default();
    let result = JudgmentResult {
        decision: "DISCARD".to_string(),
        confidence: 0.5,
        reason: "uncertain".to_string(),
    };
    assert!(!evaluate_judgment(&result, &config));
}

#[test]
fn evaluate_judgment_keeps_keep_decision() {
    let config = QualityFilterConfig::default();
    let result = JudgmentResult {
        decision: "KEEP".to_string(),
        confidence: 0.9,
        reason: "meaningful".to_string(),
    };
    assert!(!evaluate_judgment(&result, &config));
}
