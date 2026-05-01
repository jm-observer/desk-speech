# Plan 5: 前端界面与交互改造

## 前置依赖
- Plan 3
- Plan 4

## 本次目标
- 在现有页面增加“词修正”管理入口与列表。
- 识别页支持动态加载数据库中的分段结果与词修正规则。
- 新增“识别后自动复制”功能，并支持开关控制。

## 涉及文件
- 修改：`tauri-examples/non-streaming-speech-recognition-from-microphone/src/index.html`
- 修改：`tauri-examples/non-streaming-speech-recognition-from-microphone/src/main.js`
- 修改：`tauri-examples/non-streaming-speech-recognition-from-microphone/src/styles.css`

## 详细设计
- UI 改造：
  - 新增“词修正”按钮，打开规则管理弹窗。
  - 弹窗包含规则表格、新增表单、启用开关、优先级输入、删除按钮。
  - 新增“自动复制识别文本”开关（默认开启），放在主控制区。
- 数据加载策略：
  - 页面初始化并行加载：`list_correction_rules` + 最近会话 `list_sessions(1,1)`。
  - 录制中保留现有 200ms polling，并叠加 `tail_session_segments` 增量拉取，保证数据库视图一致。
  - 规则保存后刷新规则列表，并调用 `reload_correction_rules`。
- 自动复制策略：
  - 每当新增 segment 到达时，若开关开启则复制该段 `text_corrected`。
  - 增加去重：仅当 `segment.id` 或 `(start,end,text_corrected)` 变化时复制。
  - 增加节流：最小复制间隔 500ms，避免连续短段刷屏。
  - 提供状态提示：“已自动复制：<文本截断预览>”。
- 配置持久化：
  - 自动复制开关状态保存到 `localStorage`（前端本地）。
  - 词修正规则持久化在 SQLite（后端）。

## 测试案例
1. 正常路径：新增规则后页面列表即时出现，后续识别文本被修正。
2. 正常路径：新增分段时自动复制到剪贴板，关闭开关后停止自动复制。
3. 边界条件：短时间大量分段触发时，节流生效且不丢最新文本。
4. 异常路径：剪贴板写入失败时提示错误但不影响识别流程。
5. 回归：原有录制/停止/导出功能行为不变。
