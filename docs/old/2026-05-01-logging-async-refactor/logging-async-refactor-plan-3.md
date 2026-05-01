# Plan 3: 入口异步化与回归验证

## 前置依赖
- Plan 1
- Plan 2

## 本次目标
- 从 `main` 到应用启动路径统一为异步风格。
- 引入并落实 `tokio` 运行时约束，避免运行时嵌套冲突。
- 完成回归测试与修复流程闭环。

## 涉及文件
- `src-tauri/src/main.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/Cargo.toml`

## 详细设计
- 入口策略：
  - 保持 Tauri 入口 `main` 形态，不强制改为 `#[tokio::main]`。
  - 在不破坏 Tauri 约束前提下，将异步初始化收敛到 `setup`/启动钩子，并通过 runtime 调度任务。
- 运行时一致性：
  - 明确单一 runtime 责任边界，避免“外层 tokio + 内层 tauri runtime”冲突。
  - 对外提供的 command 保持 async 签名，内部避免阻塞调用。
- 回归与验证：
  - 执行 `cargo clippy --workspace -- -D warnings`
  - 执行 `cargo fmt --check --all`
  - 执行 `cargo test --workspace`

## 测试案例
- 正常路径：应用可启动，初始化成功，主要命令调用正常。
- 边界条件：初始化未完成时触发命令，返回可理解错误而非崩溃。
- 异常场景：初始化任务失败时，状态可见且可恢复（重试或重启）。
