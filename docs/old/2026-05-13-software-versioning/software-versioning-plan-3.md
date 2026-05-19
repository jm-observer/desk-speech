# Plan 3: 升级检测与兼容迁移

## 前置依赖
- Plan 1: 统一版本模型与单一真源
- Plan 2: 运行时版本暴露与前端接入

## 本次目标
- 设计“首次运行新版本”的检测机制。
- 明确应用版本与数据库迁移、配置迁移、规则版本之间的协同关系。
- 保证升级逻辑显式、可追踪，不把兼容判断散落到业务代码各处。

## 涉及文件
- `src-tauri/src/db/schema.rs`
- `src-tauri/src/db/mod.rs`
- `src-tauri/src/settings.rs`
- `src-tauri/src/config/quality_filter.rs`
- `src-tauri/src/commands/init.rs`
- `src-tauri/migrations/*.sql`（如需要补元数据表）
- `schema/app-version-info.json`
- `docs/2026-05-13-software-versioning/software-versioning.md`

## 详细设计

### 1. 首次升级检测机制
- 需要持久化一个“最近成功启动的软件版本”。
- 建议优先复用现有 `settings` 键值表，而不是新增独立表：
  - 键名：`app.last_run_version`
  - 值：应用版本字符串，如 `1.13.0`
- 启动流程：
  1. 数据库初始化与迁移完成。
  2. 读取 `app.last_run_version`。
  3. 与当前 `app_version` 比较。
  4. 若不存在，视为首次安装，`first_run_after_upgrade = false`。
  5. 若存在且不同，`first_run_after_upgrade = true`。
  6. 当启动关键流程成功后，将当前版本写回 `app.last_run_version`。

### 2. 为何不直接用应用版本驱动迁移
- 数据库迁移应由迁移编号或 schema 常量控制，因为：
  - 同一个 `app_version` 可能不涉及数据库变化。
  - 补丁版本升级不应强制触发额外迁移逻辑。
  - 多个功能可能在不同版本共同复用同一份 schema。
- 因此：
  - 数据库兼容判断使用 `schema_version`
  - 配置对象兼容判断使用 `config_schema_version`
  - 应用版本只负责“发行身份”和“用户可见升级事件”

### 3. 数据库 schema 版本来源
- 当前项目已经通过 `src-tauri/migrations/*.sql` 顺序迁移。
- 建议在 `db/schema.rs` 中维护具名常量，例如：
  - `const DB_SCHEMA_VERSION: u32 = 4;`
- 每次新增迁移时同步更新该常量，使运行时可对外暴露当前数据库结构版本。
- 注意：
  - 该版本号不是应用版本号。
  - 该版本号不需要遵循 SemVer。

### 4. 配置 schema 版本收敛
- 当前 `QualityFilterConfig.version` 已存在，但 VAD 设置和 LLM 设置仍以隐式默认值兼容。
- 建议后续统一形成“配置 schema 版本”概念：
  - 先定义全局 `CONFIG_SCHEMA_VERSION`
  - 局部配置仍可保留各自 `version` 字段，但由统一常量驱动兼容说明
- 初期实现可采用保守方案：
  - `config_schema_version` 返回当前最大/主配置版本
  - 文档中明确这只是全局配置兼容位，不等同某一个具体配置对象字段

### 5. 升级提示与副作用边界
- `first_run_after_upgrade` 只作为 UI 提示信号，不直接触发破坏性操作。
- 如需执行一次性升级逻辑，应通过显式迁移函数或具名任务完成，不得在 UI 看到升级标记后临时做隐式修补。
- 这样可以避免：
  - 前端重复渲染导致重复执行“升级副作用”
  - 版本字符串比较分散在多个模块

### 6. 失败与回滚考虑
- 若启动中途失败，不应过早写入 `app.last_run_version`，否则会把失败启动误记为成功升级。
- 建议在以下条件满足后写回：
  - 数据库初始化完成
  - 关键状态初始化完成
  - 版本信息已成功组装
- 写回失败时：
  - 记录单次 `error!`
  - 不阻断应用主流程
  - 下次启动可能再次判定为升级，属于可接受降级行为

## 测试案例
- 首次安装：
  - `app.last_run_version` 不存在，返回 `first_run_after_upgrade = false`
- 升级启动：
  - 本地存储为 `1.12.0`，当前为 `1.13.0`，返回 `true`
- 同版本重启：
  - 本地存储与当前一致，返回 `false`
- 写回失败：
  - 写入 `app.last_run_version` 失败时，应用其他启动流程不受阻断
- 边界检查：
  - 数据库 schema 版本和应用版本在代码中分别暴露，不共享同一字段
