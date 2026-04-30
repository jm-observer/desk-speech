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
    let session_id = db.create_session().unwrap();

    db.insert_segment(NewSegment {
        session_id: session_id.clone(),
        revision: 1,
        start_sec: 0.0,
        end_sec: 1.0,
        wall_start: "2026-04-29 10:00:00".to_string(),
        wall_end: "2026-04-29 10:00:01".to_string(),
        text_raw: "hello".to_string(),
    })
    .unwrap();

    let sessions = db.list_sessions(0, 1).unwrap();
    let segments = db.list_segments(&session_id, 0, 10).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(segments.len(), 1);

    let _ = std::fs::remove_file(path);
}

#[test]
fn supports_rule_version_and_tail_query() {
    let path = temp_db_path("db-tail");
    let db = SpeechDatabase::init(&path).unwrap();
    let session_id = db.create_session().unwrap();

    for i in 0..3 {
        db.insert_segment(NewSegment {
            session_id: session_id.clone(),
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
    let tails = db.tail_segments(&session_id, 1, 10).unwrap();

    assert_eq!(first_ver, 1);
    assert_eq!(latest.unwrap().0, 1);
    assert_eq!(tails.len(), 2);

    let _ = std::fs::remove_file(path);
}
