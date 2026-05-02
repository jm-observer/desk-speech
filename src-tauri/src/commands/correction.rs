use serde::Serialize;

use crate::correction::CorrectionEngine;
use crate::db::{repository::CorrectionRule, repository::NewRule, SpeechDatabase};

#[derive(Serialize)]
pub struct CorrectionRuleDto {
    pub id: i64,
    pub source: String,
    pub target: String,
    pub enabled: bool,
    pub priority: i32,
    pub updated_at: String,
}

pub async fn list_correction_rules(db: &SpeechDatabase) -> Result<Vec<CorrectionRuleDto>, String> {
    let rows = db.list_rules().await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(to_dto).collect())
}

pub async fn create_correction_rule(
    db: &SpeechDatabase,
    engine: &CorrectionEngine,
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
) -> Result<(), String> {
    validate_source(&source)?;
    db.upsert_rule(NewRule {
        source,
        target,
        priority,
        enabled,
    })
    .await
    .map_err(|e| e.to_string())?;
    reload_correction_rules(db, engine).await
}

pub async fn update_correction_rule(
    db: &SpeechDatabase,
    engine: &CorrectionEngine,
    id: i64,
    source: String,
    target: String,
    priority: i32,
    enabled: bool,
) -> Result<(), String> {
    validate_source(&source)?;
    db.update_rule(
        id,
        NewRule {
            source,
            target,
            priority,
            enabled,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    reload_correction_rules(db, engine).await
}

pub async fn delete_correction_rule(db: &SpeechDatabase, engine: &CorrectionEngine, id: i64) -> Result<(), String> {
    db.delete_rule(id).await.map_err(|e| e.to_string())?;
    reload_correction_rules(db, engine).await
}

pub async fn reload_correction_rules(db: &SpeechDatabase, engine: &CorrectionEngine) -> Result<(), String> {
    let rules = db.list_rules().await.map_err(|e| e.to_string())?;
    let checksum = engine.reload(rules.clone()).map_err(|e| e.to_string())?;
    let latest = db.get_latest_rule_version().await.map_err(|e| e.to_string())?;
    if latest.as_ref().map(|(_, c)| c != &checksum).unwrap_or(true) {
        db.bump_rule_version(checksum).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn validate_source(source: &str) -> Result<(), String> {
    if source.trim().is_empty() {
        return Err("source cannot be empty".to_string());
    }
    Ok(())
}

fn to_dto(rule: CorrectionRule) -> CorrectionRuleDto {
    CorrectionRuleDto {
        id: rule.id,
        source: rule.source,
        target: rule.target,
        enabled: rule.enabled,
        priority: rule.priority,
        updated_at: rule.updated_at,
    }
}
