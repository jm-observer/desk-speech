# Plan 1: 后端逻辑优化与配置支持

## 前置依赖
无

## 本次目标
1. 在配置项中增加 `auto_copy_mode` 字段，允许用户选择自动复制的内容类型。
2. 修改后处理任务逻辑，使其根据配置决定是否复制英文或优化中文。
3. 完成设置存储层（数据库）字段扩展与迁移，保证默认自动复制英文。

## 涉及文件
- `src-tauri/src/llm_settings.rs`: 定义配置项。
- `src-tauri/src/lib.rs`: 引用配置并执行有条件的复制逻辑。
- `src-tauri/src/*settings*` 或对应持久化模块：设置读写与默认值回填。
- `src-tauri/migrations/*`（若项目使用迁移文件）: 增加 `auto_copy_mode` 字段的 schema 迁移。

## 详细设计

### 1. 配置项定义
在 `LlmSettings` 中增加：
```rust
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub enum AutoCopyMode {
    Off,
    English,
    OptimizedZh,
}
```

并为 `LlmSettings` 提供稳定默认值：
- `auto_copy_mode` 默认值设为 `AutoCopyMode::English`。
- 对历史配置（缺失该字段）走反序列化默认值，不因字段缺失报错。

### 2. 数据库存储与迁移
- 若设置已持久化到数据库，新增 `auto_copy_mode` 字段（建议 `TEXT NOT NULL DEFAULT 'English'`）。
- 对存量记录执行回填：
  - 旧数据无该字段时，读取结果应落到 `English`。
  - 若存在旧布尔开关（如 `auto_copy=true/false`），迁移规则明确为：
    - `true -> English`
    - `false -> Off`
- 迁移需保持幂等，避免重复执行导致失败。

### 3. 逻辑修改
在 `spawn_llm_postprocess_task_v2` 中：
- 获取当前 `auto_copy_mode`。
- 模式为 `English` 时复制英文；模式为 `OptimizedZh` 时复制优化中文；`Off` 不复制。
- 增加单点日志，明确记录“已根据配置自动复制 xxx”，避免多层重复打印。

## 测试案例
- **正常路径**：配置为 `English`，识别完成后检查剪贴板是否为英文。
- **正常路径**：配置为 `OptimizedZh`，识别完成后检查剪贴板是否为优化中文。
- **正常路径**：配置为 `Off`，识别完成后检查剪贴板是否未被修改。
- **迁移验证**：旧数据库升级后，未显式配置时读取值应为 `English`。
- **边界条件**：网络延迟导致多段并发回传时，确保剪贴板反映的是最后到达的结果。
