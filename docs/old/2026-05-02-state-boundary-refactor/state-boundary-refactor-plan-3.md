# Plan 3: DB 访问边界收口

## 前置依赖
- Plan 1: 命令层异步化与状态访问统一

## 本次目标
- 明确数据库共享访问策略，减少命令层直接持有 DB 锁。
- 为后续统一读写模型和降低阻塞风险做准备。

## 涉及文件
- `src-tauri/src/lib.rs`
- `src-tauri/src/db/mod.rs`
- `src-tauri/src/db_worker.rs`
- `src-tauri/src/commands/history_api.rs`
- `src-tauri/src/commands/correction_api.rs`
- `src-tauri/src/settings.rs`

## 详细设计
- 盘点当前 DB 访问模式：
  - 写操作通过 `db_worker`
  - 多数读操作直接通过 `state.db` 获取 `SpeechDatabase`
- 评估两种方案并择一实施：
  - 方案 A：保留直接读，但把外层状态锁替换为更适合同步 DB 的同步锁或只读句柄。
  - 方案 B：引入统一 DB actor/read gateway，让命令层通过异步入口获取结果。
- 该 Plan 目标是收口边界，不额外改动前端协议或历史/纠错业务语义。

## 测试案例
- 正常路径：历史查询、纠错规则查询与更新可正常工作。
- 边界条件：数据库未初始化时，相关命令错误语义保持稳定。
- 异常路径：并发查询与后台写入并存时，不出现死锁或运行时 panic。
