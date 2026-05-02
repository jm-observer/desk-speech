# Plan 2: recording/asr 主流程下沉

## 前置依赖
- Plan 1

## 本次目标
- 将录音命令与录音核心流程（含输入流/VAD/分段识别/LLM后处理）从 `lib.rs` 迁移到 `commands/recording.rs`。
- 维持 Tauri 命令签名与现有行为一致。

## 涉及文件
- 修改：`src-tauri/src/commands/recording.rs`
- 修改：`src-tauri/src/lib.rs`

## 详细设计
- `commands/recording.rs` 承载：
  - 命令函数：`start_recording`、`stop_recording`、`clear_results`、`get_recording_state`
  - 内部流程函数：`run_recording`、`build_input_stream`、`recognize_segment`
  - LLM 后处理函数：`spawn_llm_postprocess_task_v2`、`perform_postprocess_and_copy`
  - 相关上下文结构体：`RecordingAnchor`、`RecordingRuntime`、`RecognizeContext`
- `lib.rs` 删除上述实现，仅保留数据结构、模型初始化、应用启动与模块组织。

## 测试案例
- 正常路径：命令可被注册并通过编译，录音流程逻辑不变。
- 边界条件：`already recording`、`models not ready`、`stop signal`、`audio channel disconnected` 等路径保持。
- 异常路径：模型不可用、设备不可用、LLM失败、DB队列满等分支仍可编译并保留日志/状态更新。
