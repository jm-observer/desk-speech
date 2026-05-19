#[path = "../src/llm_client.rs"]
mod llm_client;
#[path = "../src/llm_settings.rs"]
mod llm_settings;

use llm_client::{extract_json, LlmOptimizeOutput};

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
