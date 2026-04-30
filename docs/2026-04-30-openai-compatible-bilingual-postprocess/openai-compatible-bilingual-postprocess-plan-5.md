# Plan 5: 测试与验收

## 前置依赖
- Plan 1
- Plan 2
- Plan 3
- Plan 4

## 本次目标
- 完整验证异步主链路、latest-only 并发策略、三行展示和失败降级。
- 明确文档阶段验收口径与后续开发阶段修复流程要求。

## 涉及文件
- `docs/2026-04-30-openai-compatible-bilingual-postprocess/openai-compatible-bilingual-postprocess.md`
- `docs/2026-04-30-openai-compatible-bilingual-postprocess/openai-compatible-bilingual-postprocess-plan-*.md`

## 详细设计
- 方案级测试清单：
  - 主流程即时返回原始文本，异步返回优化+英文。
  - 并发场景仅最后一条落有效优化结果。
  - LLM 失败时只展示原始文本。
  - 英文复制失败不影响主流程。
- 开发阶段修复流程（实施时执行）：
  1. `cargo clippy --workspace -- -D warnings`
  2. `cargo fmt --check --all`
  3. `cargo test --workspace`

## 测试案例
- 正常路径：三行完整展示并状态闭环。
- 边界条件：高并发段落输入下 latest-only 正确生效。
- 异常场景：Provider 超时、解析失败、复制失败均可容错。
