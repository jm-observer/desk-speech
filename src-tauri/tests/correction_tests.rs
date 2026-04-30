#[path = "../src/correction.rs"]
mod correction;
#[path = "../src/db/mod.rs"]
mod db;

use correction::CorrectionEngine;
use db::repository::CorrectionRule;

fn mk_rule(id: i64, source: &str, target: &str, enabled: bool, priority: i32) -> CorrectionRule {
    CorrectionRule {
        id,
        source: source.to_string(),
        target: target.to_string(),
        enabled,
        priority,
        updated_at: "2026-04-29 11:00:00".to_string(),
    }
}

#[test]
fn applies_rules_by_priority() {
    let engine = CorrectionEngine::new();
    engine
        .reload(vec![
            mk_rule(1, "今天", "今日", true, 20),
            mk_rule(2, "今天下午", "今下午", true, 10),
        ])
        .unwrap();

    assert_eq!(engine.apply("今天下午开会"), "今下午开会");
}

#[test]
fn ignores_disabled_rules_after_reload() {
    let engine = CorrectionEngine::new();
    engine.reload(vec![mk_rule(1, "foo", "bar", true, 1)]).unwrap();
    assert_eq!(engine.apply("foo"), "bar");

    engine.reload(vec![mk_rule(1, "foo", "bar", false, 1)]).unwrap();
    assert_eq!(engine.apply("foo"), "foo");
}

#[test]
fn rejects_empty_source_rule() {
    let engine = CorrectionEngine::new();
    let err = engine.reload(vec![mk_rule(1, " ", "x", true, 1)]).unwrap_err();
    assert!(err.to_string().contains("source cannot be empty"));
}
