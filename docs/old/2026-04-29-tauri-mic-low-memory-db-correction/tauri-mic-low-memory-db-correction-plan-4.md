# Plan 4: 词修正动态配置与运行时加载

## 前置依赖
- Plan 1

## 本次目标
- 提供词修正规则（source->target）的增删改查。
- 规则变更后可在录制中动态加载，并写入数据库版本记录。
- 第一阶段明确不做 hotword boosting。

## 涉及文件
- 修改：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/lib.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/correction.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/commands/correction.rs`

## 详细设计
- 规则表达：
  - 文本替换链（按 `priority` 从小到大执行）。
  - 每条规则：`source` 非空、`target` 可空（支持删除词）。
- 非目标（本期不做）：
  - 不实现 hotword boosting。
  - 不实现外部 FST 规则编辑器。
- 运行时加载：
  - `CorrectionEngine` 启动时加载 `enabled=1` 规则。
  - 使用 `Arc<RwLock<RuleSnapshot>>`，识别线程读快照，管理线程替换快照。
  - 规则变更后计算 checksum，写入 `correction_rule_versions`。
- 应用点：
  - `recognize_segment` 得到 `text_raw` 后执行替换得到 `text_corrected`。
  - 入库同时存 `text_raw` 与 `text_corrected`。
  - UI 主显示 `text_corrected`，可选展开 `text_raw`。
- Tauri commands：
  - `list_correction_rules()`
  - `create_correction_rule(source, target, priority, enabled)`
  - `update_correction_rule(id, ...)`
  - `delete_correction_rule(id)`
  - `reload_correction_rules()`

## 测试案例
1. 正常路径：新增规则后新分段立即应用修正。
2. 正常路径：禁用规则后不再生效。
3. 边界条件：多条规则同 source 不同 priority，执行顺序正确。
4. 异常路径：空 source 被拒绝。
5. 并发场景：录制中频繁更新规则，不发生崩溃或数据竞争。
