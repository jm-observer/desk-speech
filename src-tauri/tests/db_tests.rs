#[path = "../src/db/mod.rs"]
mod db;

use db::repository::{NewRule, NewSegment};
use db::SpeechDatabase;

fn temp_db_path(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("streaming-speech-it-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

#[tokio::test]
async fn initializes_and_migrates_schema() {
    let path = temp_db_path("db-migrate");
    let db = SpeechDatabase::init(&path).await.unwrap();
    db.ensure_global_scope().await.unwrap();

    db.upsert_segment(NewSegment {
        segment_id: 1,
        revision: 1,
        start_sec: 0.0,
        end_sec: 1.0,
        wall_start: "2026-04-29 10:00:00".to_string(),
        wall_end: "2026-04-29 10:00:01".to_string(),
        text_raw: "hello".to_string(),
    })
    .await
    .unwrap();

    let segments = db.list_segments(0, 10).await.unwrap();
    assert_eq!(segments.len(), 1);

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn supports_rule_version_and_tail_query() {
    let path = temp_db_path("db-tail");
    let db = SpeechDatabase::init(&path).await.unwrap();
    db.ensure_global_scope().await.unwrap();

    for i in 0..3 {
        db.upsert_segment(NewSegment {
            segment_id: i as u64 + 1,
            revision: i as i64 + 1,
            start_sec: i as f32,
            end_sec: i as f32 + 0.5,
            wall_start: format!("2026-04-29 10:00:0{i}"),
            wall_end: format!("2026-04-29 10:00:0{}", i + 1),
            text_raw: format!("raw-{i}"),
        })
        .await
        .unwrap();
    }

    db.upsert_rule(NewRule {
        source: "foo".to_string(),
        target: "bar".to_string(),
        enabled: true,
        priority: 1,
    })
    .await
    .unwrap();

    let first_ver = db.bump_rule_version("v1".to_string()).await.unwrap();
    let latest = db.get_latest_rule_version().await.unwrap();
    let tails = db.tail_segments(1, 10).await.unwrap();

    assert_eq!(first_ver, 1);
    assert_eq!(latest.unwrap().0, 1);
    assert_eq!(tails.len(), 2);

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn upsert_same_segment_appends_full_raw_text() {
    let path = temp_db_path("db-append-raw");
    let db = SpeechDatabase::init(&path).await.unwrap();
    db.ensure_global_scope().await.unwrap();

    db.upsert_segment(NewSegment {
        segment_id: 7,
        revision: 1,
        start_sec: 1.0,
        end_sec: 1.5,
        wall_start: "2026-05-05 10:00:01".to_string(),
        wall_end: "2026-05-05 10:00:02".to_string(),
        text_raw: "第一段".to_string(),
    })
    .await
    .unwrap();
    db.upsert_segment(NewSegment {
        segment_id: 7,
        revision: 2,
        start_sec: 1.4,
        end_sec: 2.0,
        wall_start: "2026-05-05 10:00:02".to_string(),
        wall_end: "2026-05-05 10:00:03".to_string(),
        text_raw: "第二段".to_string(),
    })
    .await
    .unwrap();

    let segments = db.list_segments(0, 10).await.unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].segment_id, 7);
    assert_eq!(segments[0].revision, 2);
    assert_eq!(segments[0].text_raw, "第一段 第二段");
    assert_eq!(segments[0].start_sec, 1.0);
    assert_eq!(segments[0].end_sec, 2.0);

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn split_stage_status_and_latest_only_constraints_hold() {
    let path = temp_db_path("db-split-stages");
    let db = SpeechDatabase::init(&path).await.unwrap();
    db.ensure_global_scope().await.unwrap();

    db.upsert_segment(NewSegment {
        segment_id: 1,
        revision: 1,
        start_sec: 0.0,
        end_sec: 0.5,
        wall_start: "2026-05-01 10:00:00".to_string(),
        wall_end: "2026-05-01 10:00:01".to_string(),
        text_raw: "raw-1".to_string(),
    })
    .await
    .unwrap();
    db.upsert_segment(NewSegment {
        segment_id: 2,
        revision: 2,
        start_sec: 0.6,
        end_sec: 1.0,
        wall_start: "2026-05-01 10:00:01".to_string(),
        wall_end: "2026-05-01 10:00:02".to_string(),
        text_raw: "raw-2".to_string(),
    })
    .await
    .unwrap();

    db.update_optimize_status(1, "running".to_string()).await.unwrap();
    db.update_optimize_status(2, "running".to_string()).await.unwrap();
    db.mark_old_revisions_skipped(2).await.unwrap();
    db.update_optimize_status(2, "success".to_string()).await.unwrap();
    db.update_translate_status(2, "failed".to_string()).await.unwrap();

    let segments = db.list_segments(0, 10).await.unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].revision, 1);
    assert_eq!(segments[0].optimize_status, "failed");
    assert_eq!(segments[0].translate_status, "blocked");
    assert_eq!(segments[1].revision, 2);
    assert_eq!(segments[1].optimize_status, "success");
    assert_eq!(segments[1].translate_status, "failed");

    let impossible_combo = segments
        .iter()
        .any(|seg| seg.optimize_status == "failed" && seg.translate_status == "success");
    assert!(!impossible_combo);

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn manual_rerun_can_target_non_latest_revision() {
    let path = temp_db_path("db-manual-rerun");
    let db = SpeechDatabase::init(&path).await.unwrap();
    db.ensure_global_scope().await.unwrap();

    db.upsert_segment(NewSegment {
        segment_id: 1,
        revision: 1,
        start_sec: 0.0,
        end_sec: 0.5,
        wall_start: "2026-05-01 10:00:00".to_string(),
        wall_end: "2026-05-01 10:00:01".to_string(),
        text_raw: "raw-1".to_string(),
    })
    .await
    .unwrap();
    db.upsert_segment(NewSegment {
        segment_id: 2,
        revision: 2,
        start_sec: 0.6,
        end_sec: 1.0,
        wall_start: "2026-05-01 10:00:01".to_string(),
        wall_end: "2026-05-01 10:00:02".to_string(),
        text_raw: "raw-2".to_string(),
    })
    .await
    .unwrap();

    db.update_optimize_status(1, "pending".to_string()).await.unwrap();
    db.update_translate_status(1, "blocked".to_string()).await.unwrap();
    db.update_optimize_status(2, "running".to_string()).await.unwrap();
    db.mark_old_revisions_skipped(2).await.unwrap();

    db.upsert_optimize_result(NewSegmentResult::optimize(1, "manual optimized"))
        .await
        .unwrap();
    db.update_optimize_status(1, "success".to_string()).await.unwrap();
    db.update_translate_status(1, "pending".to_string()).await.unwrap();
    db.upsert_translate_result(NewSegmentResult::translate(1, "manual english"))
        .await
        .unwrap();
    db.update_translate_status(1, "success".to_string()).await.unwrap();

    let segments = db.list_segments(0, 10).await.unwrap();
    let manual_segment = segments.iter().find(|segment| segment.revision == 1).unwrap();
    let latest_segment = segments.iter().find(|segment| segment.revision == 2).unwrap();

    assert_eq!(manual_segment.optimize_status, "success");
    assert_eq!(manual_segment.translate_status, "success");
    assert_eq!(manual_segment.text_optimized.as_deref(), Some("manual optimized"));
    assert_eq!(manual_segment.text_english.as_deref(), Some("manual english"));
    assert_eq!(latest_segment.optimize_status, "running");

    let _ = std::fs::remove_file(path);
}

struct NewSegmentResult;

impl NewSegmentResult {
    fn optimize(revision: i64, text: &str) -> db::repository::OptimizeResultUpsert {
        db::repository::OptimizeResultUpsert {
            revision,
            text_optimized: Some(text.to_string()),
            optimize_error: None,
            optimize_started_at: None,
            optimize_finished_at: None,
        }
    }

    fn translate(revision: i64, text: &str) -> db::repository::TranslateResultUpsert {
        db::repository::TranslateResultUpsert {
            revision,
            text_english: Some(text.to_string()),
            translate_error: None,
            translate_started_at: None,
            translate_finished_at: None,
        }
    }
}
