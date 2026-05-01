# Plan 2: 并发模型改造（线程 -> 异步任务）

## 前置依赖
- Plan 1

## 本次目标
- 将业务代码中的 `std::thread::spawn` 全部改为异步任务调度。
- 明确 CPU 密集与 I/O 密集任务边界，必要时使用 `spawn_blocking`。
- 统一任务生命周期管理与停止信号处理。

## 涉及文件
- `src-tauri/src/lib.rs`
- 可能涉及：`src-tauri/src/audio_buffer.rs`、`src-tauri/src/db/*.rs`（按实际调用链）

## 详细设计
- 任务模型：
  - 现存所有 `std::thread::spawn` 调用点统一替换为 runtime 异步任务。
  - I/O 任务优先采用 `tauri::async_runtime::spawn`。
  - CPU 密集/阻塞任务采用 `tokio::task::spawn_blocking`，并在任务边界收敛错误。
- 停止机制：
  - 复用现有停止标记/通道；若当前为阻塞等待，改造成异步接收（如 `tokio::sync` 通道）。
  - 任务结束时统一发出状态事件，避免 UI 状态悬挂。
- 错误传播：
  - `JoinHandle` 结果必须显式处理，禁止静默丢弃。
  - 对取消、panic、业务错误进行区分并记录。

## 测试案例
- 正常路径：开始录音后任务成功启动并持续产生分段结果。
- 边界条件：快速开始/停止多次，不出现僵尸任务或资源重复占用。
- 异常场景：后台任务内部失败时，主流程可观测到错误并正确回收状态。
