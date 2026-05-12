# Plan 2: 后端单条手动后处理编排设计

## 前置依赖
- Plan 1: 手动后处理状态模型与接口契约

## 本次目标
- 基于现有自动后处理代码，设计一条面向指定 `revision` 的手动执行路径。
- 保证代码复用优先，不在手动链路里复制第二份 LLM 调用和状态写回逻辑。
- 设计查询、状态落库、内存态同步和并发保护的实现方式。

## 涉及文件
- `src-tauri/src/commands/history_api.rs`
- `src-tauri/src/commands/history.rs`
- `src-tauri/src/commands/recording.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/db/mod.rs`
- `src-tauri/src/db/repository.rs`
- `src-tauri/src/db_worker.rs`
- `src-tauri/src/llm_client.rs`

## 详细设计

### 1. 复用现有后处理核心函数

当前自动链路的核心实现位于：

- `spawn_llm_postprocess_task_v2`
- `perform_postprocess_and_copy`
- `update_segment_llm_state`

设计建议：

- 提取一个更通用的异步函数，例如 `run_segment_postprocess(...)`，只负责：
  - 优化调用
  - 翻译调用
  - 状态推进
  - 数据库写回
- 自动链路和手动链路都调用该函数。
- 与自动链路相关的“跳过旧 revision”“自动复制”“通知”等策略，用参数显式控制，不再硬编码在统一流程内部。

建议引入一个内部参数结构体，避免参数过多：

```rust
struct PostprocessRequest {
    revision: i64,
    input_text: String,
    allow_stale_skip: bool,
    allow_auto_copy: bool,
    trigger_source: PostprocessTriggerSource,
}
```

其中 `PostprocessTriggerSource` 可为内部枚举：

- `Automatic`
- `Manual`

该枚举仅用于日志和分支策略，不对外暴露。

### 2. 新增查询能力

手动触发前需要先查询目标记录的原始文本与当前状态。

数据库层建议补充一个按 `revision` 查询单条记录的方法，例如：

- `get_segment_by_revision(revision: i64) -> Result<Option<SegmentRow>>`

用途：

- 校验目标是否存在。
- 获取 `text_raw` 作为 LLM 输入。
- 判断当前是否已经处于 `pending/running`，用于互斥。

### 3. 命令执行流程

`manual_optimize_translate` 的建议流程：

1. 从 `AppState` 取出数据库实例与当前 `llm_settings`。
2. 查询 `revision` 对应记录。
3. 校验记录存在且 `text_raw` 非空。
4. 检查该 `revision` 是否已经在处理中。
   - 可同时检查数据库状态和内存中的 `segments` 状态。
   - 任一侧显示处理中，都拒绝再次触发。
5. 重置目标记录状态与结果：
   - 内存态：调用 `update_segment_llm_state` 或新增专用重置函数。
   - 数据库：更新状态，并清空 LLM 结果字段。
6. 启动异步任务执行统一后处理函数。
7. 命令立即返回 `Ok(())` 或等待任务完成后返回：
   - 推荐“等待任务完成后返回”，因为前端当前 API 风格以简单 `invoke` 为主，减少“命令成功提交但后台异步失败”的歧义。
   - 若后续希望与自动链路风格完全一致，也可改为“立即返回 + 后台异步执行”，但需同步补上更强的前端刷新和错误反馈。

### 4. 状态与结果落库

由于当前 `DbEvent` 主要面向自动链路，手动链路有两种实现方式：

#### 方案 A：继续复用 `DbEvent`

- 优点：状态写入路径统一。
- 缺点：`DbEvent` 目前没有“清空已有优化/翻译结果”的事件，需要扩展事件类型；而同步等待队列消费完成较困难。

#### 方案 B：手动链路直接调用 `SpeechDatabase`

- 优点：命令执行时序更清晰，适合同步等待场景。
- 缺点：会形成“自动链路走队列、手动链路直写”的双路径。

本轮建议采用方案 B，理由如下：

- 手动命令天然是显式用户操作，更适合同步返回成功/失败。
- 只要将“状态推进规则”抽到共享函数里，就不会造成业务语义分叉。
- 目前需求只针对单条记录，直写数据库的复杂度可控。

#### 直写步骤建议

1. 重置阶段：
   - `update_optimize_status(revision, "pending")`
   - `update_translate_status(revision, "blocked")`
   - 新增仓储方法清空 `text_optimized` / `text_english` 及可选错误字段
2. 优化开始：
   - `update_optimize_status(revision, "running")`
3. 优化成功：
   - `upsert_optimize_result(...)`
   - `update_optimize_status(revision, "success")`
   - `update_translate_status(revision, "pending")`
4. 翻译开始：
   - `update_translate_status(revision, "running")`
5. 翻译成功：
   - `upsert_translate_result(...)`
   - `update_translate_status(revision, "success")`

### 5. 内存态同步

当前录音页优先从 `getRecordingState` 轮询内存态；历史列表则从数据库读。

为了避免在录音进行中手动触发时出现“数据库已更新但内存仍旧显示旧值”的问题，手动链路必须同步更新内存态：

- 若 `segments` 中存在对应 `revision`，则同步改写其状态与结果字段。
- 若不存在，不视为错误，因为历史列表可能来自数据库分页，不一定都在内存中。

建议增加两个小型内部函数：

- `reset_segment_llm_state(...)`
- `set_segment_processing_state(...)`

这样可以避免在多处调用 `update_segment_llm_state` 时传入大量 `Option` 组合。

### 6. 与自动链路的差异策略

手动链路与自动链路共用同一后处理主体，但以下策略必须显式区分：

1. `allow_stale_skip`
   - 自动：`true`
   - 手动：`false`
2. `allow_auto_copy`
   - 自动：沿用设置项
   - 手动：默认 `false`
3. `mark_old_revisions_skipped`
   - 自动：执行
   - 手动：禁止执行

### 7. 日志与可观测性

日志必须带上触发来源和 `revision`，便于区分：

- `[llm][manual] start revision=...`
- `[llm][manual] optimize success revision=...`
- `[llm][manual] translate failed revision=..., err=...`

同时避免重复打印：

- 底层 `llm_client` 返回错误，不在多层重复 `error!`。
- 命令层负责一次汇总日志，数据库失败通过 `.context(...)` 提供足够上下文。

## 测试案例

1. 手动成功：目标记录存在，命令完成后数据库和内存态都更新为成功结果。
2. 处理中拒绝重入：同一 `revision` 已是 `running` 时再次调用，立即返回错误。
3. 历史 revision 不跳过：即使系统存在更大的 `revision`，手动触发旧记录仍完整执行，不被标记为 skipped。
4. 优化失败：数据库状态为 `optimize_status = failed`、`translate_status = blocked`，结果字段清空。
5. 翻译失败：数据库状态为 `optimize_status = success`、`translate_status = failed`，保留 `text_optimized`。
6. 内存缺失容错：目标记录不在 `segments` 内存列表时，手动命令仍可成功，仅依赖数据库刷新展示。
