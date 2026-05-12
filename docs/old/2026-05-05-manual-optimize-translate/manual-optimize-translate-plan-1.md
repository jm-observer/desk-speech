# Plan 1: 手动后处理状态模型与接口契约

## 前置依赖
- 无

## 本次目标
- 明确“手动优化与翻译”按钮的产品语义、目标记录标识和状态流转。
- 定义前端到 Tauri 命令层的入参、返回值和错误语义。
- 约束手动链路与自动链路的边界，确保后续实现不需要再发明第二套状态机。

## 涉及文件
- `docs/2026-05-05-manual-optimize-translate/manual-optimize-translate.md`
- `src/src/api/tauri-client.ts`
- `src/src/components/SegmentCard.tsx`
- `src-tauri/src/commands/history_api.rs`
- `src-tauri/src/commands/history.rs`
- `src-tauri/src/commands/recording.rs`

## 详细设计

### 1. 按钮语义

- 按钮名称固定为 `手动优化与翻译`。
- 点击含义：以该卡片当前对应的 `text_raw` 为输入，重新执行一次完整的“文本优化 -> 英文翻译”流程。
- 结果落点：覆盖当前 `revision` 对应的 `text_optimized`、`text_english`、`optimize_status`、`translate_status`。
- 不改变 `text_raw`，也不创建新的分段记录。

### 2. 目标标识

- 前端调用命令时传入 `revision`。
- 后端以 `revision` 查询目标记录，并据此更新 `asr_raw_records` 与 `asr_llm_results`。
- 理由：
  - `revision` 已用于状态更新与结果写回。
  - `segment_id` 在合并场景下会复用同一逻辑分段，不适合作为单次处理结果的唯一键。

### 3. 状态流转

手动链路复用现有状态，不新增数据库字段。

#### 初始状态重置

- 接收到手动触发请求后，先将目标记录重置为：
  - `optimize_status = pending`
  - `translate_status = blocked`
  - `text_optimized = null`
  - `text_english = null`
- 前端收到局部更新后，立即显示“优化中...”。

#### 执行阶段

1. 开始优化：
   - `optimize_status = running`
   - `translate_status = blocked`
2. 优化成功：
   - 保存 `text_optimized`
   - `optimize_status = success`
   - `translate_status = pending`
3. 开始翻译：
   - `translate_status = running`
4. 翻译成功：
   - 保存 `text_english`
   - `translate_status = success`

#### 失败阶段

1. 优化失败：
   - `optimize_status = failed`
   - `translate_status = blocked`
   - `text_optimized` 和 `text_english` 保持空值
2. 翻译失败：
   - `optimize_status = success`
   - `translate_status = failed`
   - 保留已成功写入的 `text_optimized`
   - `text_english` 清空

### 4. 并发与互斥规则

- 同一 `revision` 只允许存在一个手动处理中任务。
- 若用户重复点击同一条正在处理的记录，后端返回业务错误，例如 `segment is already being processed`。
- 不同 `revision` 是否允许并发处理：
  - 设计上允许，但本轮实现建议先串行化到“单命令单任务”，避免多个并发请求同时争抢同一套 LLM 配置与 UI 刷新时序。
  - 若实施时发现串行化过于保守，可在不改接口的前提下放宽为“按 revision 级别互斥”。

### 5. Tauri 命令契约

新增命令建议：

```ts
manualOptimizeTranslate(revision: number): Promise<void>
```

命令名称建议：

```rust
#[tauri::command]
async fn manual_optimize_translate(revision: i64, state: tauri::State<'_, AppState>) -> Result<(), String>
```

#### 输入约束

- `revision` 必须大于 0。
- 对应记录必须存在。
- `text_raw.trim()` 不能为空。

#### 返回语义

- 成功返回 `Ok(())`，表示“任务已成功完成”或“已成功完成并落库”，不返回大对象，结果刷新由已有读取接口承担。
- 失败返回 `Err(String)`，供前端 toast 或局部错误提示使用。

### 6. 前端最小数据契约

- `Segment` 类型无需新增字段。
- 组件层额外维护一个局部集合，如 `manualBusyRevisions: Set<number>`，用于在命令请求往返期间尽快禁用按钮。
- 后续以列表刷新后的 `optimize_status/translate_status` 作为真实状态源，避免前端局部状态长期漂移。

## 测试案例

1. 正常路径：指定存在的 `revision`，优化成功、翻译成功，状态依次从 `pending -> running -> success` 与 `blocked -> pending -> running -> success` 演进。
2. 目标不存在：传入不存在的 `revision`，命令返回错误，前端按钮恢复可点击。
3. 输入为空：目标记录 `text_raw` 为空白时拒绝执行，并返回明确错误。
4. 重复点击：同一 `revision` 处于处理中时再次触发，第二次请求被拒绝，不产生第二个并发任务。
5. 翻译失败：优化成功但翻译失败时，保留 `text_optimized`，`translate_status = failed`。
