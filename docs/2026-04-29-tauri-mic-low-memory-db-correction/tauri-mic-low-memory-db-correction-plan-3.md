# Plan 3: 识别结果入库与查询接口

## 前置依赖
- Plan 1
- Plan 2

## 本次目标
- 识别分段实时入库，录制完成后可按会话查询。
- 提供前端动态加载历史数据接口（分页 + 增量）。

## 涉及文件
- 修改：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/lib.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/commands/history.rs`

## 详细设计
- 录制流程改造：
  - `start_recording` 创建 `session_id`，写入 `sessions`。
  - 每次 `recognize_segment` 成功后，构造 `SegmentPersistEvent` 投递 DB worker。
  - `stop_recording` 时更新 `sessions.ended_at`。
- 新增 Tauri commands：
  - `list_sessions(page, page_size)`
  - `list_session_segments(session_id, page, page_size)`
  - `tail_session_segments(session_id, after_id, limit)`（增量拉取）
- 返回模型：
  - `DbSessionDto`、`DbSegmentDto`（字段与表一一对应，避免泄露内部结构体）。
- 并发与可靠性：
  - DB worker 使用有界队列；满队列时记录错误并告警（避免无限积压）。
  - 应用退出时 flush 队列，确保段落尽量落盘。

## 测试案例
1. 正常路径：录制中产生 100 段，数据库记录数与内存分段一致。
2. 边界条件：分页最后一页不足 `page_size` 返回正确。
3. 异常路径：`session_id` 不存在时返回明确错误。
4. 异常路径：DB worker 队列满时不会阻塞录制线程，并可观测告警。
