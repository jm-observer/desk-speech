use crate::versioning::AppVersionInfo;
use log::info;

#[tauri::command]
pub async fn get_app_version_info(db: tauri::State<'_, crate::db::SpeechDatabase>) -> Result<AppVersionInfo, String> {
    info!("[get_app_version_info]");
    let info = AppVersionInfo::new(&db).await.map_err(|e| e.to_string())?;
    AppVersionInfo::save_last_run_version(&db).await;
    Ok(info)
}
