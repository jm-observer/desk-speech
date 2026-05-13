# Plan 1: 统一版本模型与单一真源

## 前置依赖
无

## 本次目标
- 明确定义项目中的三类版本：应用版本、数据/schema 版本、规则版本。
- 选定应用版本的单一真源，并规定其他清单文件如何同步。
- 为后续运行时接口和升级逻辑提供稳定命名，避免语义混用。

## 涉及文件
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `package.json`
- `src-tauri/src/lib.rs`
- `src-tauri/src/commands/mod.rs`
- `src/src/api/tauri-client.ts`
- `schema/app-version-info.json`（建议新增）
- `docs/2026-05-13-software-versioning/software-versioning.md`

## 详细设计

### 1. 版本术语统一
- `app_version`
  - 表示软件发行版本。
  - 用于 UI 展示、日志上报、用户反馈、发布包命名、升级提示。
  - 格式采用 SemVer 风格字符串，例如 `1.13.0`。
- `schema_version`
  - 表示数据库 schema 或本地存储结构版本。
  - 仅服务迁移与兼容，不直接展示给普通用户。
- `config_schema_version`
  - 表示配置对象结构版本。
  - 与 `QualityFilterConfig.version` 一类字段同层，但建议未来通过统一命名收敛。
- `rule_version`
  - 表示纠错规则集合的内部版本。
  - 继续保留整数递增模式，不映射到应用版本。

### 2. 单一真源选择
- 选择 `src-tauri/Cargo.toml` 的 `[package].version` 作为 `app_version` 单一真源。
- 选择原因：
  - Rust 后端运行时代码可以零成本使用 `env!("CARGO_PKG_VERSION")`。
  - Tauri 桌面产物以 Rust crate 为核心，发版时以 Cargo 版本为主更稳定。
  - 避免前端 `package.json` 成为真源后还需额外注入到 Rust 编译流程。

### 3. 派生同步规则
- `src-tauri/tauri.conf.json.version`
  - 作为打包元数据保留，但不允许手工独立演进。
  - 每次发版时由脚本或校验命令确认与 Cargo 版本一致。
- 根目录 `package.json.version`
  - 作为工程元数据保留，继续同步到与 Cargo 一致的值。
  - 主要服务 `tauri build`、工具链和仓库可见性，不作为运行时读取源。
- 若后续新增根级 workspace `Cargo.toml`，仍以实际桌面应用 crate 的 `package.version` 为准，避免“workspace 版本”和“应用版本”不一致。

### 4. 统一协议结构
- 建议在 `schema/` 下新增 `app-version-info.json`，定义前后端共享的版本信息结构。
- 约束字段：
  - `app_version: string`
  - `app_name: string`
  - `build_profile: string`
  - `git_commit: string | null`
  - `schema_version: integer`
  - `config_schema_version: integer`
  - `first_run_after_upgrade: boolean`
- 这样可以避免前端直接根据零散接口自由扩展字段，满足“前后端结构体以 schema 为准”的约束。

### 5. 命名和边界约束
- 运行时新增的对外模型统一使用 `AppVersionInfo`，避免命名为泛化的 `VersionInfo`。
- 原有局部字段保留原义，但文档中明确：
  - `QualityFilterConfig.version` 不代表应用版本。
  - `correction_rule_versions.version` 不代表配置 schema 版本。
- 新代码中不得将 `app_version` 存入规则版本表或配置对象内部，以防职责反转。

## 测试案例
- 一致性检查：
  - 修改 Cargo 版本后，校验脚本能发现 `tauri.conf.json` 或 `package.json` 未同步。
- 协议检查：
  - `schema/app-version-info.json` 中字段与 Rust/TypeScript 结构体一一对应。
- 命名边界检查：
  - 搜索代码中对外暴露的 `version` 字段时，能够区分 `app_version`、`schema_version`、`rule_version` 的使用场景。
