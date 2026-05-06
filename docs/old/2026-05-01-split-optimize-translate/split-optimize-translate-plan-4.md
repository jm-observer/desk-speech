# Plan 4: 测试与验收设计

## 前置依赖
- Plan 1
- Plan 2
- Plan 3

## 本次目标
- 建立覆盖两阶段拆分核心风险的测试矩阵。
- 给出可执行验收标准，确保变更可上线。

## 涉及文件
- `src-tauri/tests/*.rs`
- `src-tauri/src/**/tests.rs`
- `src/**/__tests__/*.test.ts`
- `docs/2026-05-01-split-optimize-translate/split-optimize-translate.md`（状态回填）

## 详细设计
- 后端测试：
  - 单元测试：状态机流转合法性、非法状态拒绝、错误映射正确性。
  - 集成测试：模拟 LLM 响应成功/失败/超时，验证落库与事件推送。
  - 并发测试：latest-only 回写保护，旧任务回写被丢弃。
- 前端测试：
  - 组件测试：三行展示与阶段 loading/failed 文案。
  - Store 测试：增量事件合并、版本保护、复制回退逻辑。
- 验收标准：
1. 优化成功时，必须在翻译完成前可见优化结果。
2. 翻译失败时，不影响优化结果展示与复制。
3. 不出现“翻译成功但优化失败”的状态组合。
4. 修复流程三项命令全部通过：`cargo clippy --workspace -- -D warnings`、`cargo fmt --check --all`、`cargo test --workspace`。

## 测试案例
- 正常路径：端到端验证“原始 -> 优化 -> 翻译”完整闭环。
- 边界条件：空文本、超长文本、包含特殊符号文本的两阶段处理结果。
- 异常场景：优化失败、翻译失败、翻译超时、事件乱序、存储写入冲突。
