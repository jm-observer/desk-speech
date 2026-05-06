# Plan 1: 定义重试配置与常量

## 前置依赖
无

## 本次目标
在 `src-tauri/src/llm_client.rs` 中明确首轮重试机制的静态配置、语义边界和日志约束，为后续实现提供一致约束。

## 涉及文件
- `src-tauri/src/llm_client.rs`
- `docs/2026-05-04-llm-retry-mechanism/llm-retry-mechanism.md`

## 详细设计
- 在文件顶部新增具名常量，避免重试参数散落在函数体内：
  - `LLM_RETRY_MAX_ATTEMPTS: u32 = 3`：总尝试次数，包含首次执行。
  - `LLM_RETRY_DELAY: Duration = Duration::from_millis(500)`：两次尝试之间的固定等待时间。
- 采用“attempts”而非“retries”命名，减少“是否包含首次执行”的歧义。
- 本 Plan 只负责定义常量和语义，不引入配置文件或数据库字段，避免把尚未验证的策略提前外露。
- 日志边界在本 Plan 中一并约定：
  - 单次失败由重试辅助函数记录 `warn!`。
  - 最终失败由上层调用方决定是否记录 `error!`。
  - 成功路径不新增逐次日志噪音。

## 产出结果
- 一组可直接复用的重试常量。
- 一份在总览文档中可引用的重试语义定义，作为 Plan 2 和 Plan 3 的输入。

## 测试案例
- 代码检查：确认常量定义在 `llm_client.rs` 顶部且命名表达语义清晰。
- 行为约束检查：确认文档和代码对“总尝试次数”的定义一致，不出现 3 次与 4 次的歧义。
