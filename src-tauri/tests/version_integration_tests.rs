#![allow(dead_code)]

#[path = "../src/config/mod.rs"]
mod config;
#[path = "../src/db/mod.rs"]
mod db;
#[path = "../src/versioning.rs"]
mod versioning;

use db::SpeechDatabase;
use versioning::AppVersionInfo;

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
async fn app_version_info_fresh_install_is_not_upgrade() {
    let path = temp_db_path("fresh-install");
    let db = SpeechDatabase::init(&path).await.unwrap();

    let info = AppVersionInfo::new(&db).await.unwrap();
    assert!(!info.first_run_after_upgrade);
}

#[tokio::test]
async fn app_version_info_same_version_is_not_upgrade() {
    let path = temp_db_path("same-version");
    let db = SpeechDatabase::init(&path).await.unwrap();

    db.upsert_setting(
        "app.last_run_version".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    )
    .await
    .unwrap();

    let info = AppVersionInfo::new(&db).await.unwrap();
    assert!(!info.first_run_after_upgrade);
}

#[tokio::test]
async fn app_version_info_different_version_is_upgrade() {
    let path = temp_db_path("diff-version");
    let db = SpeechDatabase::init(&path).await.unwrap();

    db.upsert_setting("app.last_run_version".to_string(), "1.12.0".to_string())
        .await
        .unwrap();

    let info = AppVersionInfo::new(&db).await.unwrap();
    assert!(info.first_run_after_upgrade);
}

#[tokio::test]
async fn save_last_run_version_writes_current_version() {
    let path = temp_db_path("save-current");
    let db = SpeechDatabase::init(&path).await.unwrap();

    AppVersionInfo::save_last_run_version(&db).await;

    let result = db.get_setting("app.last_run_version".to_string()).await.unwrap();
    assert_eq!(result, Some(env!("CARGO_PKG_VERSION").to_string()));
}
