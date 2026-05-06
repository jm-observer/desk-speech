use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use anyhow::{anyhow, Result};

use crate::db::repository::CorrectionRule;

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuleSnapshot {
    pub rules: Vec<CorrectionRule>,
}

#[derive(Clone, Default)]
pub struct CorrectionEngine {
    snapshot: Arc<RwLock<RuleSnapshot>>,
}

impl CorrectionEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&self, text: &str) -> String {
        let snapshot = read_lock(&self.snapshot);

        let mut output = text.to_string();
        for rule in snapshot.rules.iter().filter(|rule| rule.enabled) {
            output = output.replace(&rule.source, &rule.target);
        }
        output
    }

    pub fn reload(&self, mut rules: Vec<CorrectionRule>) -> Result<String> {
        if rules.iter().any(|r| r.source.trim().is_empty()) {
            return Err(anyhow!("source cannot be empty"));
        }

        rules.sort_by_key(|rule| (rule.priority, rule.id));
        let checksum = checksum_rules(&rules);
        let snapshot = RuleSnapshot { rules };
        let mut guard = write_lock(&self.snapshot);
        *guard = snapshot;

        Ok(checksum)
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
    fn apply_by_priority() {
        let engine = CorrectionEngine::new();
        engine
            .reload(vec![mk_rule(1, "abc", "x", true, 20), mk_rule(2, "ab", "y", true, 10)])
            .unwrap();
        assert_eq!(engine.apply("abc"), "yc");
    }

    #[test]
    fn reject_empty_source() {
        let engine = CorrectionEngine::new();
        let err = engine.reload(vec![mk_rule(1, " ", "x", true, 10)]).unwrap_err();
        assert!(err.to_string().contains("source cannot be empty"));
    }
}
