# Plan 2: 录音/导出热路径状态拆分

## 前置依赖
- Plan 1: 命令层异步化与状态访问统一

## 本次目标
- 识别录音实时路径与导出路径中不能 `await` 的共享状态。
- 将这部分状态从 Tokio 锁中拆分为同步锁或更合适的同步结构。
- 让录音热路径不再依赖 Tokio 锁的 `blocking_*` 接口。

## 涉及文件
- `src-tauri/src/lib.rs`
- `src-tauri/src/commands/recording.rs`
- `src-tauri/src/commands/export.rs`
- `src-tauri/src/audio_buffer.rs`

## 详细设计
- 将 `AppState` 中供录音热路径访问的字段单独归类：
  - `segments`
  - `recorded_audio`
  - `current_session_id`
  - `start_wall_clock`
  - `start_instant`
  - 视实现需要再评估 `recognizer`、`vad`
- 这类字段优先改为 `std::sync::{Mutex, RwLock}` 或必要时配合 `Atomic*`。
- 保证以下场景不出现 `.await`：
  - CPAL 回调
  - 录音循环核心路径
  - 纯同步辅助函数（如 LLM 状态更新辅助函数）
- 导出命令按最终状态容器选择配套锁实现，避免同步导出命令跨层访问 Tokio 锁。

## 测试案例
- 正常路径：开始录音、停止录音、清空结果、导出 WAV/SRT 正常工作。
- 边界条件：无录音数据时导出继续返回既有错误。
- 异常路径：录音结束后后台异步后处理仍能正确更新段状态，不触发锁类型不匹配或 panic。
