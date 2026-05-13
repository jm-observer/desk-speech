# Plan 2: 运行时版本暴露与前端接入

## 前置依赖
- Plan 1: 统一版本模型与单一真源

## 本次目标
- 设计后端版本查询命令和前端类型定义。
- 让前端能够稳定读取并展示当前软件版本，而不依赖硬编码或构建时手填。
- 保持现有初始化与设置接口职责单一，不把版本信息揉入无关响应。

## 涉及文件
- `src-tauri/src/commands/init.rs`
- `src-tauri/src/commands/mod.rs`
- `src-tauri/src/lib.rs`
- `src/src/api/tauri-client.ts`
- `src/src/App.tsx`
- `src/src/components/SettingsModal.tsx`
- `schema/app-version-info.json`
- `docs/2026-05-13-software-versioning/software-versioning.md`

## 详细设计

### 1. 后端命令设计
- 新增独立命令：`get_app_version_info() -> Result<AppVersionInfo, String>`
- 放置建议：
  - 若保持模块聚合度，可放入 `src-tauri/src/commands/init.rs`
  - 更清晰的做法是新增 `src-tauri/src/commands/app_info.rs`，但本轮若追求最小变更，也可先与 init 命令同模块
- 返回内容：
  - `app_version`：来自 `env!("CARGO_PKG_VERSION")`
  - `app_name`：常量 `StreamSpeech` 或由配置读取后的稳定值
  - `build_profile`：由 `cfg!(debug_assertions)` 推导为 `debug` / `release`
  - `git_commit`：本轮可固定为 `None`
  - `schema_version`：来自数据库迁移常量
  - `config_schema_version`：来自配置迁移常量
  - `first_run_after_upgrade`：由 Plan 3 的本地元数据判断

### 2. 结构体定义位置
- 建议在 `src-tauri/src/commands/init.rs` 或独立 `app_info.rs` 中定义 `AppVersionInfo` 响应结构。
- 结构体字段显式命名，不复用 `InitStatus`：
  - `InitStatus` 继续只承载模型加载状态。
  - `AppVersionInfo` 单独承载软件元数据。
- 这样可避免前端轮询初始化状态时附带无关字段，降低接口耦合。

### 3. 前端 API 设计
- 在 `src/src/api/tauri-client.ts` 中新增：
  - `export interface AppVersionInfo { ... }`
  - `getAppVersionInfo: () => invoke<AppVersionInfo>('get_app_version_info')`
- 前端不再从 `package.json`、构建常量或文案手工读取版本字符串，统一经 Tauri 命令获取。

### 4. 前端展示策略
- 建议最小展示入口：
  - 设置弹窗底部：展示 `StreamSpeech v1.13.0`
  - 关于信息区域：追加 `schema_version` 和 `build_profile` 的折叠式开发信息
- 主界面不强制长期占位展示版本，避免干扰主任务流。
- 若 `first_run_after_upgrade = true`，前端可在设置页或欢迎提示中展示“已升级到 x.y.z”，但具体文案由 Plan 3 定义。

### 5. 日志和排障价值
- 启动日志建议追加一条单次 `info!`：
  - `app_version`
  - `build_profile`
  - `schema_version`
- 这样用户报障时可同时从日志和 UI 获取相同版本信息，减少定位歧义。

## 测试案例
- 正常路径：
  - 前端启动后调用 `get_app_version_info`，能拿到非空 `app_version`。
- 一致性路径：
  - UI 展示版本与 `src-tauri/Cargo.toml` 一致。
- 边界路径：
  - `git_commit` 为空时，前端仍能正常渲染版本区域。
- 职责边界检查：
  - `get_init_status` 响应结构不因本 Plan 膨胀。
