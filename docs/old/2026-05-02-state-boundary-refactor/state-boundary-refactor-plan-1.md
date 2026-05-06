# Plan 1: 命令层异步化与状态访问统一

## 前置依赖
- 无

## 本次目标
- 将命令层中依赖 Tokio 锁的接口统一改为 `async fn`。
- 消除设置、设备、初始化、历史、纠错等命令层中的 `blocking_read`、`blocking_write`、`blocking_lock`。
- 保持现有前端调用协议不变，仅调整后端命令实现边界。

## 涉及文件
- `src-tauri/src/settings.rs`
- `src-tauri/src/commands/init.rs`
- `src-tauri/src/commands/device.rs`
- `src-tauri/src/commands/history_api.rs`
- `src-tauri/src/commands/correction_api.rs`
- `src-tauri/src/lib.rs`

## 详细设计
- 将命令层按“是否访问 Tokio 锁”划分：
  - 只要读取或写入 `AppState` 中的 Tokio 锁字段，就改为 `async fn`。
  - 命令内部统一使用 `.read().await`、`.write().await`、`.lock().await`。
- 保持 `AppState` 当前结构不大改，避免在同一 Plan 内混入热路径拆分。
- 对涉及数据库的命令：
  - 先维持当前“拿到 DB 句柄后直接调用 repository”的模式。
  - 仅把外层 `state.db.blocking_lock()` 改为 `state.db.lock().await`。
- 对设置相关命令：
  - `get_settings`、`apply_settings` 统一改为 `async fn`。
  - `apply_settings` 内部状态比较、写回缓存、数据库持久化都在异步命令内完成。
- 对初始化状态命令：
  - `get_init_status` 改为异步读取 `init_error`，避免同步命令读取 Tokio 锁。

## 测试案例
- 正常路径：打开设置弹窗，`get_settings`、`list_llm_models`、`get_selected_device`、`get_init_status` 可正常返回。
- 正常路径：历史列表与纠错规则列表仍可返回数据。
- 边界条件：数据库未初始化时，历史/纠错/设置命令继续返回原有错误。
- 异常路径：LLM 配置非法时，`list_llm_models` 继续返回配置校验错误，不出现 Tokio panic。
