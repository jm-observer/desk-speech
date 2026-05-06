# Plan 2: 实现通用的重试逻辑函数

## 前置依赖
Plan 1

## 本次目标
设计并实现一个内部异步辅助函数，用于封装后处理链路的重试逻辑，并保持调用方式足够简单，便于 `optimize_text()` 与 `translate_text()` 复用。

## 涉及文件
- `src-tauri/src/llm_client.rs`

## 详细设计
- 实现一个私有泛型异步函数 `retry_task`，签名可保持如下方向：
  - `async fn retry_task<T, F, Fut>(task_name: &str, max_attempts: u32, delay: Duration, task: F) -> Result<T, String>`
  - `F: FnMut() -> Fut`
  - `Fut: Future<Output = Result<T, String>>`
- 增加 `task_name` 参数而非在日志中硬编码，便于区分 `optimize_text` 和 `translate_text` 的失败来源。
- 核心控制流：
  1. 从第 1 次尝试开始循环执行闭包。
  2. 成功时立即返回结果，不做额外包装。
  3. 失败时记录当前错误并判断是否已达到 `max_attempts`。
  4. 若尚未达到上限，记录 `warn!`，包含任务名、当前尝试次数、最大次数和错误信息。
  5. 使用 `tokio::time::sleep(delay).await` 等待后进入下一轮。
  6. 到达上限仍失败时，直接返回最后一次错误，避免吞掉最接近根因的信息。
- 约束点：
  - 不使用阻塞休眠。
  - 不在函数内做错误分类；所有 `Err(String)` 一视同仁，保持首版行为简单。
  - 不在函数内记录最终 `error!`，避免与上层 `perform_postprocess_and_copy()` 重复。
- 可维护性要求：
  - 函数保持私有，避免将未稳定的内部抽象暴露为公共接口。
  - 若为测试需要提升可见性，优先使用 `pub(crate)` 或测试模块直接访问，而不是公开给外部 crate。

## 测试案例
- 正常路径：闭包首次成功时立即返回，不发生等待与后续重试。
- 部分失败路径：前 N 次失败、后一次成功时，返回最终成功结果且调用次数符合预期。
- 彻底失败路径：连续失败达到上限后返回最后一次错误。
- 参数边界：`max_attempts` 若传入 `0`，实现中需防御性处理；推荐在函数内部归一化为至少 1 次执行，避免出现“完全不执行”的隐式分支。
