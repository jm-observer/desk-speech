# lib-slimming

## 时间
- 创建时间：2026-05-02
- 最后更新：2026-05-02

## 项目现状
- `src-tauri/src/lib.rs` 当前约 1600 行，包含运行入口、状态定义、录音/识别主流程、LLM 后处理、DB 异步写入等多类职责。
- 历史与纠错 API 已经拆回对应 `commands/*_api.rs`，但 `db worker` 和 `recording/asr` 仍集中在 `lib.rs`。

## 整体目标
- 在不改变行为的前提下，继续收敛 `lib.rs`：
  - 将 `db worker` 相关类型与执行循环迁出到独立模块。
  - 将 `recording/asr` 主流程迁入 `commands/recording.rs`，命令与核心流程同文件维护。
- 保持现有前端命令接口与数据库/LLM行为一致。

## Plan 拆分
- Plan 1（先执行，进行中）：`db worker` 模块化
  - 内容：迁移 `DbEvent` 与 `start_db_worker` 到独立模块（如 `db_worker.rs`），`lib.rs` 仅保留调用。
  - 依赖：无
  - 状态：进行中
- Plan 2（后执行，待开始）：`recording/asr` 主流程下沉
  - 内容：将录音驱动、输入流构建、VAD 分段识别、LLM 后处理相关实现迁入 `commands/recording.rs`，移除 `lib.rs` 中对应实现。
  - 依赖：Plan 1
  - 状态：待开始

执行顺序：Plan 1 -> Plan 2

## 风险与待定项
- 风险：跨模块迁移后，`use` 与可见性边界容易出现遗漏，需依赖 `clippy` 与编译校验。
- 风险：录音流程涉及线程、channel、状态共享，迁移时必须避免逻辑改写。
