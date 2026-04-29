# Plan 6: 测试与发布验证

## 前置依赖
- Plan 1-5

## 本次目标
- 建立覆盖后端与前端关键路径的测试与验证流程。
- 在合入前完成长录制内存验证、数据库迁移验证与自动复制验证。

## 涉及文件
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/tests/db_tests.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/tests/correction_tests.rs`
- 新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/src-tauri/tests/rolling_buffer_tests.rs`
- 可选新增：`tauri-examples/non-streaming-speech-recognition-from-microphone/docs/manual-test-checklist.md`

## 详细设计
- 自动化测试：
  - DB schema 初始化与迁移测试。
  - 修正规则优先级与热更新一致性测试。
  - 10 分钟环形缓存边界测试（600s/601s/超长）。
- 手工回归清单：
  - 连续录制 30 分钟，观察进程内存稳定在目标区间。
  - 录制中新增/禁用规则，验证后续段落即时生效。
  - 自动复制开启/关闭、失败重试、节流行为符合预期。
  - 重启应用后历史会话与规则仍可加载。
- 完成门禁：
  - 在 workspace 根执行：
    - `cargo clippy --workspace -- -D warnings`
    - `cargo fmt --check --all`
    - `cargo test --workspace`

## 测试案例
1. 正常路径：三项门禁全部通过。
2. 性能场景：30 分钟录制过程中内存不随时长线性增长。
3. 容错场景：数据库文件损坏时给出可恢复提示并允许重建。
4. 交互场景：自动复制异常不阻断识别与入库链路。
