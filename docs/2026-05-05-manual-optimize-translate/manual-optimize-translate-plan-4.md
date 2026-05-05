# Plan 4: 测试与验收设计

## 前置依赖
- Plan 1: 手动后处理状态模型与接口契约
- Plan 2: 后端单条手动后处理编排设计
- Plan 3: 前端按钮与刷新交互设计

## 本次目标
- 为“手动优化与翻译”建立可执行的测试清单与验收标准。
- 覆盖正常路径、并发路径、失败路径和录音中交互路径。
- 明确修复流程执行口径，确保实施完成后可通过现有 workspace 质量门禁。

## 涉及文件
- `src-tauri/tests/correction_tests.rs`
- `src-tauri/tests/db_tests.rs`
- `src-tauri/src/commands/history_api.rs`
- `src-tauri/src/commands/recording.rs`
- `src/src/components/SegmentCard.tsx`
- `src/src/App.tsx`

## 详细设计

### 1. 后端单元/集成测试

优先补充 Rust 测试，覆盖以下场景：

1. `manual_optimize_translate` 成功路径
   - 构造一条带 `text_raw` 的记录
   - mock 或替代 LLM 响应为固定优化文本和英文文本
   - 校验数据库中的状态和结果字段
2. 优化失败
   - 让优化调用返回错误
   - 校验 `optimize_status = failed`
   - 校验 `translate_status = blocked`
3. 翻译失败
   - 优化返回成功、翻译返回失败
   - 校验 `text_optimized` 被保留
   - 校验 `translate_status = failed`
4. 重复触发拒绝
   - 第一次任务将记录置为 `running`
   - 第二次调用返回业务错误
5. 历史记录可重跑
   - 选择非最新 `revision`
   - 校验不会被 `mark_old_revisions_skipped` 逻辑影响

### 2. 前端交互测试

如果项目当前未建立前端测试框架，本轮至少输出手工验证清单；若已有轻量测试能力，则补充以下用例：

1. 按钮可见性
   - 已完成记录显示按钮
   - 无 `revision` 记录按钮禁用
2. 点击后禁用
   - 调用进行中时按钮展示 `处理中...`
3. 成功后刷新
   - mock API 成功后，卡片显示新的 `text_optimized` / `text_english`
4. 失败后恢复
   - mock API 失败后，按钮恢复为可点击
5. 与现有复制按钮共存
   - 新按钮不会遮挡或破坏现有 hover 操作区布局

### 3. 手工验收场景

建议在桌面端按以下顺序回归：

1. 非录音状态下对历史分段点击“手动优化与翻译”
   - 观察按钮进入忙碌态
   - 观察优化文本和翻译文本刷新
2. 对同一条处理中分段连续点击
   - 第二次点击无效或给出明确错误
3. 构造翻译失败
   - 观察优化结果保留、翻译区域显示失败提示
4. 录音进行中对已有分段触发手动处理
   - 观察轮询未中断
   - 观察对应卡片状态与结果一致
5. 手动触发成功后检查剪贴板
   - 确认默认不会被自动覆盖，除非实施阶段明确保留该行为
6. 手动触发成功后检查系统通知
   - 确认默认不弹“识别完成”通知

### 4. 验收标准

以下条件全部满足才视为功能达标：

1. 前端卡片存在可用的“手动优化与翻译”入口。
2. 手动触发仅影响目标 `revision`，不会误伤其他分段状态。
3. 同一记录不会因重复点击启动多个并发任务。
4. 优化失败与翻译失败的 UI 表现符合既有状态语义。
5. 自动识别链路行为不回归，录音时的自动后处理仍正常工作。
6. 代码实现完成后，修复流程全部通过：
   - `cargo clippy --workspace -- -D warnings`
   - `cargo fmt --check --all`
   - `cargo test --workspace`

## 测试案例

1. Happy path：手动触发后完整生成优化文本和英文翻译。
2. Busy path：处理中重复点击被拒绝。
3. Error path：优化失败、翻译失败两类错误都能正确落状态。
4. Recording path：录音中手动触发不破坏自动轮询和自动识别流程。
5. Regression path：不点击按钮时，现有自动“优化 -> 翻译”链路行为与之前一致。
