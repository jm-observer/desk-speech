use crate::versioning::AppVersionInfo;
use log::info;

#[tauri::command]
pub async fn get_app_version_info(
    app_name: String,
    db: tauri::State<'_, crate::db::SpeechDatabase>,
) -> Result<AppVersionInfo, String> {
    info!("[get_app_version_info]");
    AppVersionInfo::new(app_name, &db).map_err(|e| e.to_string())
}
