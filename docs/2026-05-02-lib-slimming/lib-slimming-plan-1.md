# Plan 1: db worker 模块化

## 前置依赖
- 无

## 本次目标
- 将 `DbEvent` 和 `start_db_worker` 从 `lib.rs` 迁移到新模块，降低 `lib.rs` 职责密度。
- 保持事件枚举、队列容量、落库逻辑与日志行为不变。

## 涉及文件
- 新增：`src-tauri/src/db_worker.rs`
- 修改：`src-tauri/src/lib.rs`

## 详细设计
- 在 `db_worker.rs` 中定义：
  - `pub(crate) const DB_EVENT_QUEUE_CAPACITY`
  - `pub(crate) enum DbEvent`
  - `pub(crate) fn start_db_worker(db: SpeechDatabase) -> SyncSender<DbEvent>`
- `lib.rs` 中：
  - 增加 `mod db_worker;`
  - `AppState.db_writer` 改用 `SyncSender<db_worker::DbEvent>`
  - 调用点改为 `db_worker::start_db_worker(db.clone())`
  - 删除原 `DbEvent` 与 `start_db_worker` 定义

## 测试案例
- 正常路径：应用可编译，启动后创建 DB writer 成功。
- 回归路径：录音期间分段写库与状态更新事件仍可入队与消费（通过现有测试/编译链路校验）。
- 异常路径：worker join 错误日志路径保持可编译并可触发（结构不变）。
