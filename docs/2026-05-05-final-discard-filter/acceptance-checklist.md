# Plan 4: 验收清单

## 1. 数据库层验收

### 1.1 查询过滤
- [ ] `list_segments` 不返回 `is_discarded = true` 的记录
- [ ] `tail_segments` 不返回 `is_discarded = true` 的记录
- [ ] `get_segment_by_revision` 仍返回已丢弃记录（用于事件处理）

### 1.2 落库字段
- [ ] 规则层丢弃后写入：`is_discarded=true`, `discard_source="rule"`, `quality_check_status="discarded"`
- [ ] LLM 层丢弃后写入：`is_discarded=true`, `discard_source="llm"`, `discard_confidence`, `quality_check_status="discarded"`
- [ ] LLM 保留后写入：`is_discarded=false`, `discard_source="llm"`, `quality_check_status="kept"`
- [ ] 判定失败后写入：`quality_check_status="check_failed"`

### 1.3 幂等与一致性
- [ ] 同一 `revision` 重复触发 ready 不产生并发判定
- [ ] 判定过程中新追加文本会回退到 `not_ready`

---

## 2. 后端判定链路验收

### 2.1 状态机流转
| 步骤 | 预期状态 | 验证方式 |
|------|----------|----------|
| 初始 | `not_ready` | 日志/内存状态 |
| 终态触发 | `ready` | `set_segment_finalization_state` |
| 判定中 | `checking` | 日志 `[finalization]` |
| 规则命中 | `discarded` | 日志 + 数据库 |
| LLM 判定 | `llm_checking` → `kept`/`discarded` | 日志 + 数据库 |
| 判定失败 | `check_failed` | 日志 + 数据库 |

### 2.2 规则层测试
- [ ] 短词（<3 字符）被丢弃
- [ ] 语气词（`ok`, `嗯`, `啊` 等）被丢弃
- [ ] 单姓名（`张三`, `李明`）被丢弃
- [ ] 高重复词（`啊啊啊啊`）被丢弃
- [ ] 有意义文本（`今天天气不错`）被保留

### 2.3 LLM 层测试
- [ ] 高置信度 DISCARD（≥0.65）→ 丢弃
- [ ] 低置信度 DISCARD（<0.65）→ 保留（防误杀）
- [ ] KEEP 决策 → 保留
- [ ] 解析失败 → `check_failed`

### 2.4 终态触发条件
- [ ] 显式 `stream_end` 事件触发
- [ ] 静默 10000ms（10 秒）兜底触发
- [ ] `ready`/`checking` 期间新文本追加 → 回退到 `not_ready`

---

## 3. 前端交互验收

### 3.1 事件驱动移除
- [ ] 收到 `segment_discarded` 事件后立即从列表移除
- [ ] 使用 `segment_id` 匹配移除
- [ ] 使用 `revision` 匹配移除（降级方案）
- [ ] 移除后列表排序正确

### 3.2 轮询兜底
- [ ] 事件丢失时，下一轮 `tail_segments` 查询不返回已丢弃记录
- [ ] 事件先到、列表后刷新：移除结果不回弹

### 3.3 交互稳定性
- [ ] 移除正在选中/查看的卡片不会导致空引用报错
- [ ] 收到未知 `revision` 的丢弃事件，不报错仅记录 debug 日志

---

## 4. 端到端手工验收

### 4.1 样本集测试

| 样本 | 文本 | 预期结果 | 判定来源 |
|------|------|----------|----------|
| A | `ok` | 丢弃 | 规则 |
| B | `张三` | 丢弃 | 规则 |
| C | `明天下午三点和客户开会` | 保留 | LLM |
| D | `嗯...` | 丢弃 | 规则 |
| E | `把这个问题分成三步解决` | 保留 | LLM |

### 4.2 验收步骤

1. **启动应用**，确认模型初始化完成
2. **开始录音**，注入或录制样本
3. **等待优化翻译完成**，观察终态判定触发
4. **验证丢弃条目即时移除**（前端 UI）
5. **验证历史查询不返回已丢弃条目**（`list_segments`/`tail_segments`）
6. **检查日志**中是否保留判定来源和置信度

---

## 5. 指标与观测

上线后至少统计以下指标：

| 指标 | 说明 | 告警阈值 |
|------|------|----------|
| `discard_rate` | 被丢弃分段占比 | > 40% |
| `rule_hit_rate` | 规则层命中占比 | > 60% |
| `llm_discard_rate` | LLM 判定丢弃占比 | > 20% |
| `check_failed_rate` | 判定失败占比 | > 5% |
| `user_restore_request_count` | 用户反馈误杀次数 | > 10/天 |

---

## 6. 测试案例清单

| 编号 | 测试场景 | 预期结果 | 状态 |
|------|----------|----------|------|
| TC-01 | 低信息量样本被稳定丢弃 | 规则层或 LLM 层判定为 DISCARD | ☐ |
| TC-02 | 高信息量样本稳定保留 | 判定为 KEEP | ☐ |
| TC-03 | 无 VAD 结束事件时，静默 10000ms 触发判定 | 进入 `ready` 状态 | ☐ |
| TC-04 | 判定失败不阻断主流程 | 记录 `check_failed` | ☐ |
| TC-05 | 丢弃事件与轮询并存时无 UI 抖动 | 移除结果一致 | ☐ |
| TC-06 | 指标日志字段完整，支持按 `revision` 追溯 | 日志包含所有字段 | ☐ |
| TC-07 | 同一 `revision` 并发触发只执行一次判定 | 幂等保护生效 | ☐ |
| TC-08 | 前端收到不存在 `revision` 事件不崩溃 | 仅记录 debug 日志 | ☐ |

---

## 7. 验收通过标准

- [ ] 所有单元测试通过
- [ ] 所有端到端手工验收步骤完成
- [ ] 指标日志字段完整
- [ ] 无 P0/P1 级别 bug
- [ ] 连续 24 小时运行无异常

---

**验收人：** _______________  
**验收日期：** _______________  
**验收结论：** ☐ 通过  ☐ 有条件通过  ☐ 不通过
