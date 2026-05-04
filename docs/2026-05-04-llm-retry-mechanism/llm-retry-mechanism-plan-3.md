# Plan 3: 重构 `optimize_text` 和 `translate_text`

## 前置依赖
Plan 2

## 本次目标
将 `optimize_text()` 与 `translate_text()` 的完整 LLM 后处理链路接入 `retry_task`，在不改变外部接口的前提下提升容错能力。

## 涉及文件
- `src-tauri/src/llm_client.rs`
- `src-tauri/src/commands/recording.rs`

## 详细设计
- `optimize_text()` 重构方式：
  - 将 `chat_json_completion()`、`extract_json()`、`serde_json::from_value()`、字段提取这条链路收敛到一个异步闭包中。
  - 通过 `retry_task("optimize_text", ...)` 执行该闭包。
  - 闭包成功返回 `String`，避免把 `LlmOptimizeOutput` 泄露到重试函数之外。
- `translate_text()` 重构方式：
  - 与 `optimize_text()` 保持完全相同的控制流结构，避免两个函数后续演化出不同的失败语义。
  - 通过 `retry_task("translate_text", ...)` 执行完整链路。
- 错误处理约束：
  - 函数签名保持 `Result<String, String>` 不变，调用方 `perform_postprocess_and_copy()` 无需修改错误分支结构。
  - 每次尝试失败不在 `optimize_text()` / `translate_text()` 内额外记录 `error!`，统一由 `retry_task()` 记录 `warn!`，最终失败交由调用方记录。
- 与调用方的协同：
  - `recording.rs` 中现有 `error!("llm postprocess failed: {}")` 与 `error!("llm translate failed: {}")` 保持不变，继续作为最终失败日志出口。
  - 文档中需明确：引入重试后，单次业务失败的出现时间会延后，调用方无需调整状态机，只需接受“失败判定更晚但更稳定”的变化。

## 数据流
1. 调用方进入 `optimize_text()` / `translate_text()`。
2. `retry_task()` 发起一次完整尝试。
3. 尝试内部依次完成请求、提取 JSON、反序列化。
4. 任一步骤失败则整次尝试失败，由 `retry_task()` 决定是否继续。
5. 成功则返回最终文本；达到上限仍失败则返回最后一次错误。

## 测试案例
- 正常路径：模拟一次成功响应，确认返回文本与既有语义一致。
- 请求失败后恢复：前几次 `chat_json_completion()` 返回错误，最后一次成功，确认函数最终成功。
- 解析失败后恢复：返回内容存在额外包裹或字段不匹配，确认会被视为可重试失败。
- 最终失败：连续失败达到上限后，调用方仍收到 `Err(String)`，且日志边界不重复。
