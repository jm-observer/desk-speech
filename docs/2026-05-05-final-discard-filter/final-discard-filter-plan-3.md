# Plan 3: 前端移除协议与交互

## 前置依赖
- Plan 1
- Plan 2

## 本次目标
- 定义后端向前端发送“已丢弃”结果的协议。
- 明确前端收到信号后的移除策略与一致性保障。
- 避免事件丢失导致 UI 与数据库状态不一致。

## 涉及文件
- `src-tauri/src/commands/recording.rs`
- `src-tauri/src/main.rs`
- `src/src/store/useAppStore.ts`
- `src/src/api/tauri-client.ts`
- `src/src/components/SegmentCard.tsx`

## 详细设计

### 1. 后端事件协议

新增 Tauri event（建议名称）：`segment_discarded`。

载荷建议：
- `revision: number`
- `segment_id: string`
- `decision: "DISCARD"`
- `reason: string`
- `source: "rule" | "llm"`
- `confidence: number | null`
- `occurred_at_ms: number`

触发时机：
- 数据库更新 `is_discarded = true` 成功后立即发送。

### 2. 前端处理策略

- 在 store 层订阅 `segment_discarded`。
- 收到事件后立即按 `revision` 从当前列表移除。
- 若当前正在展示该条详情，自动关闭详情视图并显示轻提示“已过滤低价值识别内容”。

### 3. 一致性保障

- 事件是“即时反馈”，轮询/刷新是“最终一致性兜底”。
- `list_segments`、`tail_segments` 默认过滤 `is_discarded = true`。
- 若事件丢失，下一轮轮询仍会将该条从列表中消失。

### 4. 回退策略

- 前端收到未知 `revision` 的丢弃事件时，仅记录 debug 日志，不报错。
- 若事件处理异常，保留轮询兜底，不阻断其他事件消费。

## 测试案例

1. 收到 `segment_discarded` 后，列表立即移除对应 `revision`。
2. 事件先到、列表后刷新：移除结果不回弹。
3. 未订阅成功时，通过下一轮 `tail_segments` 仍能正确移除。
4. 收到不存在 `revision` 事件，不引发前端崩溃。
5. 移除时若用户聚焦该条卡片，UI 正常回退到列表态。
