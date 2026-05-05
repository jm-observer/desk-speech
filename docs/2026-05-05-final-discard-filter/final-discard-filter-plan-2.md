# Plan 2: 丢弃判定协议与后端编排

## 前置依赖
- Plan 1

## 本次目标
- 定义“规则兜底 + LLM 判定”的组合策略。
- 明确判定输入输出、数据库落库字段与错误处理策略。
- 约束判定结果如何驱动后续前端移除。

## 涉及文件
- `src-tauri/src/llm_client.rs`
- `src-tauri/src/commands/recording.rs`
- `src-tauri/src/db/schema.rs`
- `src-tauri/src/db/repository.rs`
- `src-tauri/src/model_registry.rs`

## 详细设计

### 1. 判定输入

以单条 `revision` 组装输入：
- `text_raw`
- `text_optimized`（若无则为空）
- `text_english`（若无则为空）
- 可选上下文：最近一条相邻分段的时间戳差（仅用于辅助，不作为硬门槛）

模型主输入建议优先 `text_optimized`，其次 `text_raw`，翻译文本用于辅助 disambiguation。

### 2. 规则层（前置）

在调用 LLM 前先执行轻量规则，命中即直接 `DISCARD`：
- 归一化后字符长度 `< 3`。
- 仅由语气词/填充词组成（如：`ok`、`嗯`、`啊`、`呃`、`嗯嗯`）。
- 仅单个姓名/称呼模式（如“张三”“王老师”），且不包含动词或实义动作。
- 高重复低信息（同一 token 重复占比 >= 0.8，长度 <= 8）。

规则未命中再进入 LLM。

### 3. LLM 输出协议

LLM 必须返回 JSON：
- `decision`: `KEEP` | `DISCARD`
- `confidence`: `0.0..1.0`
- `reason`: 简短中文说明（<= 30 字）

解析失败或字段缺失视为判定失败，进入 `check_failed` 并按重试策略执行。

### 4. 判定阈值

- 若 `decision=DISCARD` 且 `confidence >= 0.65`，执行丢弃。
- 若 `decision=DISCARD` 但 `confidence < 0.65`，保守改判为 `KEEP`（防误杀）。
- 若 `decision=KEEP`，直接保留。

### 5. 落库字段（软删除）

建议在结果表新增字段：
- `is_discarded: bool`
- `discard_reason: Option<String>`
- `discard_source: Option<String>`（`rule` / `llm`）
- `discard_confidence: Option<f32>`
- `quality_check_status: String`（与 Plan 1 状态机对齐）

丢弃后不物理删除记录，只更新标记。

### 6. 错误边界

- 规则层永不抛错，异常时返回“未命中规则”。
- LLM 请求失败：记录一次错误日志并进入 `check_failed`。
- 禁止在多层重复打印同一错误；命令边界统一打印。

## 测试案例

1. 命中规则层（`ok`）直接丢弃，不调用 LLM。
2. 规则未命中，LLM 返回 `DISCARD` 且高置信度，最终丢弃。
3. LLM 返回低置信度 `DISCARD`，最终保留。
4. LLM 返回非法 JSON，状态进入 `check_failed`。
5. 丢弃后字段完整落库：`is_discarded/reason/source/confidence`。
