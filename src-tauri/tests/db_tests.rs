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

#[test]
fn initializes_and_migrates_schema() {
    let path = temp_db_path("db-migrate");
    let db = SpeechDatabase::init(&path).unwrap();
    db.ensure_global_scope().unwrap();

    db.upsert_segment(NewSegment {
        segment_id: 1,
        revision: 1,
        start_sec: 0.0,
        end_sec: 1.0,
        wall_start: "2026-04-29 10:00:00".to_string(),
        wall_end: "2026-04-29 10:00:01".to_string(),
        text_raw: "hello".to_string(),
    })
    .unwrap();

    let segments = db.list_segments(0, 10).unwrap();
    assert_eq!(segments.len(), 1);

    let _ = std::fs::remove_file(path);
}

#[test]
fn supports_rule_version_and_tail_query() {
    let path = temp_db_path("db-tail");
    let db = SpeechDatabase::init(&path).unwrap();
    db.ensure_global_scope().unwrap();

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
        .unwrap();
    }

    db.upsert_rule(NewRule {
        source: "foo".to_string(),
        target: "bar".to_string(),
        enabled: true,
        priority: 1,
    })
    .unwrap();

    let first_ver = db.bump_rule_version("v1").unwrap();
    let latest = db.get_latest_rule_version().unwrap();
    let tails = db.tail_segments(1, 10).unwrap();

    assert_eq!(first_ver, 1);
    assert_eq!(latest.unwrap().0, 1);
    assert_eq!(tails.len(), 2);

    let _ = std::fs::remove_file(path);
}

#[test]
fn split_stage_status_and_latest_only_constraints_hold() {
    let path = temp_db_path("db-split-stages");
    let db = SpeechDatabase::init(&path).unwrap();
    db.ensure_global_scope().unwrap();

    db.upsert_segment(NewSegment {
        segment_id: 1,
        revision: 1,
        start_sec: 0.0,
        end_sec: 0.5,
        wall_start: "2026-05-01 10:00:00".to_string(),
        wall_end: "2026-05-01 10:00:01".to_string(),
        text_raw: "raw-1".to_string(),
    })
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
    .unwrap();

    db.update_optimize_status(1, "running").unwrap();
    db.update_optimize_status(2, "running").unwrap();
    db.mark_old_revisions_skipped(2).unwrap();
    db.update_optimize_status(2, "success").unwrap();
    db.update_translate_status(2, "failed").unwrap();

    let segments = db.list_segments(0, 10).unwrap();
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
