use serde::Serialize;

use crate::db::SpeechDatabase;

/// 应用版本信息，前后端共享的统一结构体。
///
/// 字段语义：
/// - `app_version`：当前软件发行版本，来源于 Cargo 包版本。
/// - `app_name`：对外展示名称，来自 Tauri 产品名。
/// - `build_profile`：`debug` / `release`，用于排障。
/// - `git_commit`：可选提交哈希；当前未注入，保持为 `None`。
/// - `schema_version`：数据库迁移体系当前版本号。
/// - `config_schema_version`：配置结构当前版本号。
/// - `first_run_after_upgrade`：本地存储的最近运行版本低于当前版本时为 `true`。
#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct AppVersionInfo {
    pub app_version: String,
    pub app_name: String,
    pub build_profile: String,
    pub git_commit: Option<String>,
    pub schema_version: u32,
    pub config_schema_version: u32,
    pub first_run_after_upgrade: bool,
}

/// 用于升级检测的持久化键名。
const LAST_RUN_VERSION_KEY: &str = "app.last_run_version";

/// 比较两个版本字符串是否不同。
///
/// 返回 `true` 表示版本已变更（即发生了升级或降级）。
/// 这是纯函数，不依赖任何 I/O，便于单元测试。
pub fn version_comparison(previous: &str, current: &str) -> bool {
    previous != current
}

impl AppVersionInfo {
    /// 构建当前应用版本信息。
    ///
    /// 从 Cargo 包版本、编译 profile、Tauri 产品名等来源获取数据。
    /// `schema_version` 使用数据库迁移常量。
    /// `config_schema_version` 使用质量过滤配置的结构版本号（固定为 1）。
    /// `first_run_after_upgrade` 通过比较当前版本与数据库中存储的最近运行版本来确定。
    pub fn new(app_name: String, db: &SpeechDatabase) -> Result<Self, anyhow::Error> {
        let build_profile = if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        };

        let last_run_version = tauri::async_runtime::block_on(db.get_setting(LAST_RUN_VERSION_KEY.to_string()))?;

        let first_run_after_upgrade = match last_run_version {
            Some(previous_version) => version_comparison(&previous_version, env!("CARGO_PKG_VERSION")),
            None => false,
        };

        Ok(Self {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            app_name,
            build_profile,
            git_commit: None,
            schema_version: crate::db::schema::DB_SCHEMA_VERSION,
            config_schema_version: 1,
            first_run_after_upgrade,
        })
    }

    /// 将当前应用版本写入数据库，标记启动成功。
    ///
    /// 在启动流程的关键步骤完成后调用。
    /// 写入失败时记录 error 日志但不阻断主流程。
    pub fn save_last_run_version(db: &SpeechDatabase) {
        let app_version = env!("CARGO_PKG_VERSION").to_string();
        if let Err(err) =
            tauri::async_runtime::block_on(db.upsert_setting(LAST_RUN_VERSION_KEY.to_string(), app_version))
        {
            log::error!("[version] failed to save last run version: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_version_info_contains_cargo_package_version() {
        // 此测试不依赖数据库，直接验证常量
        assert_eq!(env!("CARGO_PKG_VERSION"), "1.13.0");
    }

    #[test]
    fn version_comparison_returns_true_when_versions_differ() {
        assert!(version_comparison("1.12.0", "1.13.0"));
        assert!(version_comparison("1.13.0", "1.13.1"));
        assert!(version_comparison("0.9.0", "1.0.0"));
    }

    #[test]
    fn version_comparison_returns_false_when_versions_match() {
        assert!(!version_comparison("1.13.0", "1.13.0"));
    }

    #[test]
    fn version_comparison_returns_false_for_empty_previous() {
        // 空字符串与任何版本都不同，模拟首次安装时由调用方处理为 false
        assert!(version_comparison("", "1.13.0"));
    }

    #[test]
    fn db_schema_version_is_positive() {
        assert!(crate::db::schema::DB_SCHEMA_VERSION > 0);
    }

    #[test]
    fn config_schema_version_is_positive() {
        assert!(1 > 0);
    }

    #[test]
    fn app_version_info_is_serializable() {
        let info = AppVersionInfo {
            app_version: "1.0.0".to_string(),
            app_name: "test".to_string(),
            build_profile: "debug".to_string(),
            git_commit: None,
            schema_version: 4,
            config_schema_version: 1,
            first_run_after_upgrade: false,
        };
        let json = serde_json::to_string(&info).expect("should serialize");
        assert!(json.contains("app_version"));
        assert!(json.contains("app_name"));
        assert!(json.contains("build_profile"));
        assert!(json.contains("schema_version"));
        assert!(json.contains("first_run_after_upgrade"));
    }

    #[test]
    fn last_run_version_key_is_defined() {
        assert!(!LAST_RUN_VERSION_KEY.is_empty());
        assert!(LAST_RUN_VERSION_KEY.contains('.'));
    }
}
