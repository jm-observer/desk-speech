# Plan 1: 数据模型与存储层设计

## 前置依赖
- 无

## 本次目标
- 定义满足“识别结果持久化 + 词修正配置”的数据库结构。
- 设计统一存储层接口，避免 Tauri command 直接操作 SQL。
- 明确迁移策略、错误处理与并发访问策略。

## 涉及文件
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/db/mod.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/db/schema.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/db/repository.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/migrations/0001_init.sql`
- 修改：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/src/lib.rs`

## 详细设计
- 数据库文件位置：`AppData/<app-name>/speech_history.db`。
- 连接生命周期：应用启动时初始化单例连接池（若使用 `rusqlite` 则用 `Arc<Mutex<Connection>>`；若后续切到 `sqlx` 可替换）。
- 表结构：
  - `sessions`
    - `id TEXT PRIMARY KEY`（UUID）
    - `started_at TEXT NOT NULL`
    - `ended_at TEXT NULL`
    - `sample_rate INTEGER NOT NULL DEFAULT 16000`
    - `channel_count INTEGER NOT NULL DEFAULT 1`
    - `created_at TEXT NOT NULL`
  - `segments`
    - `id INTEGER PRIMARY KEY AUTOINCREMENT`
    - `session_id TEXT NOT NULL`
    - `start_sec REAL NOT NULL`
    - `end_sec REAL NOT NULL`
    - `wall_start TEXT NOT NULL`
    - `wall_end TEXT NOT NULL`
    - `text_raw TEXT NOT NULL`
    - `text_corrected TEXT NOT NULL`
    - `created_at TEXT NOT NULL`
    - 索引：`idx_segments_session_time(session_id, start_sec)`
  - `correction_rules`
    - `id INTEGER PRIMARY KEY AUTOINCREMENT`
    - `source TEXT NOT NULL`
    - `target TEXT NOT NULL`
    - `enabled INTEGER NOT NULL DEFAULT 1`
    - `priority INTEGER NOT NULL DEFAULT 100`
    - `updated_at TEXT NOT NULL`
    - 唯一约束：`UNIQUE(source, target)`
  - `correction_rule_versions`
    - `version INTEGER PRIMARY KEY`
    - `checksum TEXT NOT NULL`
    - `created_at TEXT NOT NULL`
- 存储层接口：
  - `create_session() -> SessionId`
  - `close_session(session_id)`
  - `insert_segment(NewSegment)`
  - `list_segments(session_id, page, page_size)`
  - `upsert_rule(NewRule)` / `delete_rule(rule_id)` / `list_rules()`
  - `bump_rule_version(checksum)` / `get_latest_rule_version()`
- 并发策略：
  - 录制线程只投递事件到 mpsc 队列。
  - 独立 DB worker 消费事件并写库，避免音频线程阻塞。
- 错误处理：
  - 存储层返回 `anyhow::Result<T>`。
  - SQL 错误加 `.context("...")`，上层统一转换为用户可读错误。

## 测试案例
1. 正常路径：初始化数据库后表与索引创建成功。
2. 正常路径：插入 session/segment/rule 并可查询。
3. 边界条件：重复 upsert 同一规则，命中唯一约束后更新而非崩溃。
4. 异常路径：数据库文件不可写时初始化失败并返回上下文错误。
5. 异常路径：非法分页参数（page_size=0）被显式拒绝。
