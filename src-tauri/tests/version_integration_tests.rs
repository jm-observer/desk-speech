#[path = "../src/db/mod.rs"]
mod db;

use db::SpeechDatabase;

fn temp_db_path(suffix: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "streaming-speech-version-it-{suffix}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

#[tokio::test]
async fn upgrade_detection_fresh_install_returns_false() {
    let path = temp_db_path("fresh-install");
    let db = SpeechDatabase::init(&path).await.unwrap();

    let result = db.get_setting("app.last_run_version".to_string()).await.unwrap();
    assert!(result.is_none(), "fresh install should have no last_run_version");
}

#[tokio::test]
async fn upgrade_detection_same_version_returns_false() {
    let path = temp_db_path("same-version");
    let db = SpeechDatabase::init(&path).await.unwrap();

    db.upsert_setting(
        "app.last_run_version".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    )
    .await
    .unwrap();

    let result = db.get_setting("app.last_run_version".to_string()).await.unwrap();
    assert_eq!(result, Some(env!("CARGO_PKG_VERSION").to_string()));
}

#[tokio::test]
async fn upgrade_detection_different_version_returns_true() {
    let path = temp_db_path("diff-version");
    let db = SpeechDatabase::init(&path).await.unwrap();

    db.upsert_setting("app.last_run_version".to_string(), "1.12.0".to_string())
        .await
        .unwrap();

    let result = db.get_setting("app.last_run_version".to_string()).await.unwrap();
    assert_eq!(result, Some("1.12.0".to_string()));
}
