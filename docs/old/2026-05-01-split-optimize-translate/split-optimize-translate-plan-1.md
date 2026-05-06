# Plan 1: 两阶段流程与状态模型设计

## 前置依赖
- 无

## 本次目标
- 建立“优化 -> 翻译”两阶段状态机，明确状态定义、流转约束和终态组合。
- 统一数据模型，支持独立存储优化与翻译结果及其错误信息。

## 涉及文件
- `schema/` 下与后处理结果相关的 schema 文件
- `src-tauri/migrations/0002_split_optimize_translate.sql`（新增迁移文件）
- `src-tauri/src/**/model*.rs`
- `src-tauri/src/**/entity*.rs`
- `src-tauri/src/**/repository*.rs`（查询/写入语句适配新字段）
- `src/**` 前端类型定义（若有共享类型映射）

## 详细设计
- 状态定义：
  - `optimize_status`: `pending | running | success | failed`
  - `translate_status`: `blocked | pending | running | success | failed`
- 流转规则：
  - 初始：`optimize=pending`, `translate=blocked`
  - 优化开始：`optimize=running`
  - 优化成功：`optimize=success`, `translate=pending`
  - 优化失败：`optimize=failed`, `translate=blocked`（终态）
  - 翻译开始：`translate=running`
  - 翻译成功：`translate=success`
  - 翻译失败：`translate=failed`
- 结果字段建议：
  - `raw_text`（原始识别）
  - `optimized_text`（优化结果，可空）
  - `translated_text_en`（英文翻译结果，可空）
  - `optimize_error`、`translate_error`（结构化错误摘要，可空）
  - `optimize_started_at`/`finished_at`、`translate_started_at`/`finished_at`
- 一致性约束：
  - `translated_text_en` 非空前提：`optimized_text` 必须非空。
  - 禁止出现 `optimize=failed` 且 `translate=success` 的非法组合。
  - 更新采用条件写入（基于段落 id + 版本戳/序号）防止旧任务回填覆盖。

## 数据库迁移设计

### 现有表结构问题
- `asr_raw_records.opt_status` 是单一状态字段（`pending|running|done|failed|skipped`），无法表达两阶段独立状态。
- `asr_llm_results` 的 `text_optimized` 和 `text_english` 均为 `NOT NULL`，无法表达"优化完成但翻译未开始"的中间态。
- 缺少分阶段错误信息和时间戳字段。

### 迁移文件：`0002_split_optimize_translate.sql`

```sql
-- 1. asr_raw_records：拆分单一 opt_status 为双阶段状态
ALTER TABLE asr_raw_records ADD COLUMN optimize_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE asr_raw_records ADD COLUMN translate_status TEXT NOT NULL DEFAULT 'blocked';
-- 历史数据兼容：将已有 opt_status 映射到新字段
UPDATE asr_raw_records SET optimize_status = CASE
    WHEN opt_status = 'done' THEN 'success'
    WHEN opt_status = 'failed' THEN 'failed'
    WHEN opt_status = 'running' THEN 'running'
    WHEN opt_status = 'skipped' THEN 'skipped'
    ELSE 'pending'
END;
UPDATE asr_raw_records SET translate_status = CASE
    WHEN opt_status = 'done' THEN 'success'
    WHEN opt_status = 'failed' THEN 'blocked'
    ELSE 'blocked'
END;
-- opt_status 保留但不再使用，SQLite 不支持 DROP COLUMN（旧版本）

-- 2. asr_llm_results：允许翻译字段为空（优化完成但翻译未开始）
-- SQLite 不支持 ALTER COLUMN，需重建表
CREATE TABLE asr_llm_results_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    text_optimized TEXT,          -- 可空：优化未完成时为 NULL
    text_english TEXT,            -- 可空：翻译未完成时为 NULL
    optimize_error TEXT,          -- 新增：优化错误摘要
    translate_error TEXT,         -- 新增：翻译错误摘要
    optimize_started_at TEXT,     -- 新增
    optimize_finished_at TEXT,    -- 新增
    translate_started_at TEXT,    -- 新增
    translate_finished_at TEXT,   -- 新增
    created_at TEXT NOT NULL,
    FOREIGN KEY(session_id) REFERENCES sessions(id),
    UNIQUE(session_id, revision)
);
INSERT INTO asr_llm_results_new (id, session_id, revision, text_optimized, text_english, created_at)
    SELECT id, session_id, revision, text_optimized, text_english, created_at FROM asr_llm_results;
DROP TABLE asr_llm_results;
ALTER TABLE asr_llm_results_new RENAME TO asr_llm_results;
```

### 历史数据兼容策略
- 旧记录 `opt_status='done'` → 映射为 `optimize_status='success'` + `translate_status='success'`（因为旧流程总是一起完成）。
- 旧记录 `opt_status='failed'` → 映射为 `optimize_status='failed'` + `translate_status='blocked'`。
- 读取时：若 `optimize_status` / `translate_status` 列不存在（极老版本），回退到 `opt_status` 映射。
- `opt_status` 列保留不删除（SQLite 3.35.0 以下不支持 DROP COLUMN），代码中不再读写此列。

### Repository 层适配
- `insert_llm_result()` → 改为分阶段写入：`upsert_optimize_result()` 和 `upsert_translate_result()`。
- `update_opt_status()` → 拆分为 `update_optimize_status()` 和 `update_translate_status()`。
- 查询语句中原来 `WHERE opt_status = 'done'` → 改为 `WHERE optimize_status = 'success'`。

## 测试案例
- 正常路径：优化成功后翻译成功，状态按顺序推进到双 success。
- 边界条件：优化输出为空字符串时判定失败并阻断翻译。
- 异常场景：优化失败后收到翻译回写请求时应拒绝写入并记录告警。
- 兼容路径：读取历史无分阶段字段数据时，按默认值映射为可展示状态。
