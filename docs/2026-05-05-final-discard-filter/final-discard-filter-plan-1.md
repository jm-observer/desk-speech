# Plan 1: 终态判定时机与状态机设计

## 前置依赖
- 无

## 本次目标
- 定义“分段何时进入终态判定”的可执行规则。
- 设计最小状态机，确保每个分段只判定一次且可追踪。
- 给出在缺乏明确 VAD stream 语义时的默认阈值策略。

## 涉及文件
- `docs/2026-05-05-final-discard-filter/final-discard-filter.md`
- `src-tauri/src/commands/recording.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/db/schema.rs`

## 详细设计

### 1. 终态触发条件

定义 `segment_finalization` 触发条件，满足其一即进入“待判定”：

1. 上游显式结束事件：
- 收到当前分段的 `stream_end` 或等价结束信号。

2. 静默兜底超时：
- 当前分段最后一次文本更新时间后，持续 `FINALIZE_SILENCE_MS = 10000` 毫秒（10 秒）未再追加文本。
- 且该分段已完成优化与翻译（或两者达到失败终态）。

说明：
- 10000ms（10 秒）作为当前默认值，优先降低长句与停顿场景误触发；后续可通过配置化调参。
- 若上游后续补发文本且该分段已进入“待判定”，需取消本轮判定任务并回到处理中状态。

### 2. 终态判定前置门槛

仅当满足以下条件才执行丢弃判定：
- `text_raw` 非空白。
- 后处理不再运行中：`optimize_status`、`translate_status` 均不为 `running/pending`。
- 当前记录尚未完成丢弃判定（幂等保护）。

### 3. 状态机扩展

在现有分段状态之上新增判定状态字段（建议枚举）：
- `not_ready`：未达到终态。
- `ready`：达到终态，等待判定。
- `checking`：判定进行中。
- `keep`：判定保留。
- `discard`：判定丢弃。
- `check_failed`：判定异常（解析失败、请求失败等）。

状态流转：
- `not_ready -> ready -> checking -> keep|discard|check_failed`
- 若 `ready/checking` 期间出现新追加文本：回退到 `not_ready`。

### 4. 幂等与去重

- 以 `revision` 作为判定任务唯一键。
- 同一 `revision` 在 `checking` 状态时禁止重复入队。
- 若因重试需要再次判定，必须由显式重试入口触发，避免后台无限循环。

### 5. 默认时间规则（按当前理解）

- `FINALIZE_SILENCE_MS = 10000`
- `CHECK_RETRY_MAX = 1`（仅一次快速重试，用于瞬时网络波动）
- `CHECK_RETRY_BACKOFF_MS = 400`

这些值必须提取为具名常量，禁止散落 magic number。

## 测试案例

1. 显式结束事件到达后立即进入 `ready`，并触发判定。
2. 无结束事件但静默超过 10000ms，进入 `ready`。
3. 进入 `ready` 后又追加新文本，状态回到 `not_ready`，本轮判定取消。
4. 同一 `revision` 重复触发 ready，不产生并发 `checking`。
5. 判定任务异常后进入 `check_failed`，不自动无限重试。
