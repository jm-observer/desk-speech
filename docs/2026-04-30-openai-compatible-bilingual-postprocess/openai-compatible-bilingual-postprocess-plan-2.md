# Plan 2: 存储模型与状态机设计

## 前置依赖
- Plan 1

## 本次目标
- 定义原始文本表与优化结果表分离模型。
- 定义状态机与 latest-only 并发语义，不考虑旧数据兼容。

## 涉及文件
- `schema/*.json`
- `src-tauri/src/**/model*.rs`
- `src-tauri/src/**/repository*.rs`

## 详细设计
- 分表设计：
  - `asr_raw_records`：保存每段原始文本。
  - `asr_llm_results`：保存有效优化结果与英文翻译。
- 原始记录状态字段：
  - `opt_status`: `pending | running | done | skipped | failed`
- latest-only 规则：
  - 会话内维护递增 `revision`。
  - 非最新 `revision` 的任务在发送前/返回后均可被判定为 `skipped`。
  - 仅最新任务允许写入 `asr_llm_results` 并推送前端。
- 旧数据策略：
  - 不做历史兼容与迁移逻辑设计，按新结构直接运行。

## 测试案例
- 正常路径：最新段落状态从 `pending` 到 `done`。
- 边界条件：连续快速输入时前序段落被正确标记 `skipped`。
- 异常场景：任务失败时状态标记 `failed` 且保留原始文本链路。
