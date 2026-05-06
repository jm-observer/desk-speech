# Plan 5: 提示词与阈值配置化

## 前置依赖
- Plan 2
- Plan 3

## 本次目标
- 将 LLM 终态判定提示词从硬编码改为可配置。
- 将置信度阈值、规则词表与静默判定窗口收敛为统一配置项。
- 提供前端可见且可校验的配置入口，避免“改代码调策略”。

## 涉及文件
- `schema/`（新增或更新质量判定配置 schema）
- `src-tauri/src/config/`（新增质量判定配置加载与校验）
- `src-tauri/src/llm_client.rs`
- `src-tauri/src/commands/recording.rs`
- `src-tauri/src/model_registry.rs`
- `src/src/api/tauri-client.ts`
- `src/src/store/useAppStore.ts`
- `src/src/components/`（新增或改造配置面板）

## 详细设计

### 1. 配置模型与默认值

新增 `QualityFilterConfig`（命名可在实施时微调），建议包含：
- `llm_prompt_template: String`
- `discard_confidence_threshold: f32`（默认 `0.65`）
- `silence_window_ms: u64`（默认 `10000`）
- `filler_tokens: Vec<String>`（语气词白名单）
- `single_name_patterns: Vec<String>`（单姓名/称呼模式）
- `repeat_ratio_threshold: f32`（默认 `0.8`）
- `enabled: bool`（总开关，默认 `true`）

要求：
- 所有字段提供默认值，防止旧配置缺字段导致启动失败。
- 数值字段做边界校验（如 `0.0..=1.0`、`silence_window_ms >= 1000`）。

### 2. 后端加载与生效策略

- 启动时加载配置并注入判定链路上下文，避免函数层层透传过多参数。
- 判定时按“配置优先、默认值兜底”读取阈值与词表。
- 当 `enabled=false` 时跳过终态丢弃判定，仅保留原有链路与日志提示。

### 3. 提示词模板协议

`llm_prompt_template` 采用占位符模板，最小支持：
- `{{text_raw}}`
- `{{text_optimized}}`
- `{{text_english}}`

约束：
- 渲染后文本为空时回退到内置默认模板。
- 模板渲染失败进入 `check_failed`，并记录一次可追溯错误日志。

### 4. 前端配置入口

- 新增“终态过滤配置”面板，至少支持：
- 编辑提示词模板。
- 调整置信度阈值与静默窗口。
- 维护语气词列表（增删）。
- 前端提交前做基础校验，后端再次强校验并返回结构化错误。

### 5. 一致性与版本管理

- 配置 schema 增加 `version` 字段，升级时做兼容迁移。
- 前后端共用 schema 定义，禁止字段独立演进。
- 配置保存后广播 `quality_filter_config_updated` 事件，提示前端刷新展示状态。

## 测试案例

1. 未提供任何配置时，系统使用默认值并正常判定。
2. 阈值从 `0.65` 调整到 `0.80` 后，边界样本判定结果按预期变化。
3. 提示词模板缺少占位符或渲染失败时，进入兜底模板并记录错误。
4. `enabled=false` 时不触发丢弃逻辑，前端不再收到 `segment_discarded`。
5. 非法配置（如阈值 `1.5`）被后端拒绝并返回明确错误信息。
