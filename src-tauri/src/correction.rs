use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::{anyhow, Result};

use crate::db::repository::CorrectionRule;

/// Stateless validator/checksummer for correction rules. The remote client no
/// longer applies corrections itself (the server owns ASR); this only validates
/// rule edits and produces a checksum for DB rule-version bookkeeping.
#[derive(Clone, Default)]
pub struct CorrectionEngine;

impl CorrectionEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn reload(&self, mut rules: Vec<CorrectionRule>) -> Result<String> {
        if rules.iter().any(|r| r.source.trim().is_empty()) {
            return Err(anyhow!("source cannot be empty"));
        }

        rules.sort_by_key(|rule| (rule.priority, rule.id));
        Ok(checksum_rules(&rules))
    }
}

fn checksum_rules(rules: &[CorrectionRule]) -> String {
    let mut hasher = DefaultHasher::new();
    for rule in rules {
        rule.id.hash(&mut hasher);
        rule.source.hash(&mut hasher);
        rule.target.hash(&mut hasher);
        rule.enabled.hash(&mut hasher);
        rule.priority.hash(&mut hasher);
        rule.updated_at.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_rule(id: i64, source: &str, target: &str, enabled: bool, priority: i32) -> CorrectionRule {
        CorrectionRule {
            id,
            source: source.to_string(),
            target: target.to_string(),
            enabled,
            priority,
            updated_at: "2026-04-29 10:00:00".to_string(),
        }
    }

    #[test]
    fn reject_empty_source() {
        let engine = CorrectionEngine::new();
        let err = engine.reload(vec![mk_rule(1, " ", "x", true, 10)]).unwrap_err();
        assert!(err.to_string().contains("source cannot be empty"));
    }

    #[test]
    fn checksum_is_stable_regardless_of_input_order() {
        let engine = CorrectionEngine::new();
        let a = engine
            .reload(vec![mk_rule(1, "abc", "x", true, 20), mk_rule(2, "ab", "y", true, 10)])
            .unwrap();
        let b = engine
            .reload(vec![mk_rule(2, "ab", "y", true, 10), mk_rule(1, "abc", "x", true, 20)])
            .unwrap();
        assert_eq!(a, b);
    }
}
