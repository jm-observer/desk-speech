# Plan 2: 后端编排与接口改造设计

## 前置依赖
- Plan 1

## 本次目标
- 将后处理执行器改造为串行两阶段编排。
- 明确每阶段请求构造、超时控制、错误传播与回写策略。

## 涉及文件
- `src-tauri/src/lib.rs`（`CombinedSettings` 结构体、`get_settings`/`apply_settings` 命令）
- `src-tauri/src/llm_settings.rs`（`LlmSettings` 结构体新增双提示词字段）
- `src-tauri/src/llm_client.rs`（`postprocess_text()` 拆分为两个函数）
- `src-tauri/src/**/pipeline*.rs`
- `src-tauri/src/**/service*.rs`
- `src-tauri/src/**/repository*.rs`（`upsert_setting`/`get_setting` 新 key）
- `src/src/api/tauri-client.ts`（`AppSettings` 接口新增字段）

## 详细设计
- 编排流程：
1. 接收原始文本并落库。
2. 启动优化任务，写入 `optimize=running`。
3. 优化成功后立即回写 `optimized_text` 并推送前端增量事件。
4. 基于 `optimized_text` 启动翻译任务，写入 `translate=running`。
5. 翻译成功后回写 `translated_text_en` 并推送二次事件。
- 请求拆分策略：
  - 优化请求提示词只描述”文本润色/纠错/去口语噪音”，明确禁止翻译输出。
  - 翻译请求提示词只描述”忠实翻译为英文”，输入来源固定为优化文本。
- **双系统提示词配置设计**：
  - 现有 `prompt_template` 单字段拆分为 `optimize_prompt_template` + `translate_prompt_template`。
  - **`LlmSettings` 结构体改造**（`src-tauri/src/llm_settings.rs`）：
    ```rust
    pub struct LlmSettings {
        pub provider_url: String,
        pub api_key: String,
        pub selected_model: String,
        pub optimize_prompt_template: String,  // 原 prompt_template 拆分
        pub translate_prompt_template: String,  // 新增
    }
    ```
  - **默认值**：
    - `optimize_prompt_template`: `”你是一个中文转写后处理助手。输入是语音识别文本，请修正错别字、去除口语噪音（如”嗯””啊”等）、补全标点，保持原意不扩写。返回 JSON：{\”text_optimized\”:\”...\”}。”`
    - `translate_prompt_template`: `”你是一个中译英翻译助手。输入是已优化的中文文本，请忠实翻译为英文，不添加解释或注释。返回 JSON：{\”text_english\”:\”...\”}。”`
  - **数据库存储**（`app_settings` 表新增 key）：
    - `llm.optimize_prompt_template`
    - `llm.translate_prompt_template`
    - 旧 key `llm.prompt_template` 保留但不再读取；启动时若新 key 不存在则从旧 key 推导或使用默认值。
  - **`CombinedSettings` 适配**（`src-tauri/src/lib.rs`）：
    - 移除 `prompt_template: String`
    - 新增 `optimize_prompt_template: String` + `translate_prompt_template: String`
  - **`apply_settings` 命令适配**：
    ```rust
    db.upsert_setting(“llm.optimize_prompt_template”, &new_llm_settings.optimize_prompt_template)?;
    db.upsert_setting(“llm.translate_prompt_template”, &new_llm_settings.translate_prompt_template)?;
    ```
  - **`llm_client.rs` 拆分**：
    - `postprocess_text()` 拆分为：
      - `optimize_text(settings, input_text) -> Result<String, String>`：使用 `optimize_prompt_template`，期望 JSON `{“text_optimized”:”...”}`。
      - `translate_text(settings, optimized_text) -> Result<String, String>`：使用 `translate_prompt_template`，期望 JSON `{“text_english”:”...”}`。
    - 两个函数各自构建独立的 system message + user message，各自要求 `ResponseFormat::JsonObject`。
  - **前端类型适配**（`src/src/api/tauri-client.ts`）：
    ```typescript
    export interface AppSettings {
      // ...existing fields...
      optimize_prompt_template: string;  // 替换原 prompt_template
      translate_prompt_template: string; // 新增
    }
    ```
  - **向后兼容**：启动加载时，若数据库无 `llm.optimize_prompt_template`，检查是否存在旧 `llm.prompt_template`；若存在则将其作为 `optimize_prompt_template` 的初始值，`translate_prompt_template` 使用默认值。
- 超时与重试：
  - 优化与翻译使用独立超时常量（如 `OPTIMIZE_TIMEOUT_SECS`、`TRANSLATE_TIMEOUT_SECS`）。
  - 默认不自动重试；如后续启用，需按阶段配置最大重试次数。
- 错误处理：
  - 优化失败：记录 `optimize_error`，停止后续翻译，返回可恢复状态。
  - 翻译失败：记录 `translate_error`，保留优化结果，流程整体标记为部分成功。
- 并发控制：
  - 维持 latest-only 原则：仅允许最新段落的后处理结果回填。
  - 回写前校验段落版本，失配则丢弃并记录 `info!` 级别日志。
- 日志与可观测性：
  - 每阶段记录开始、结束、耗时、结果状态。
  - 错误日志只在边界层输出一次，避免重复打印。

## 测试案例
- 正常路径：两阶段均成功，验证两次事件推送顺序与内容。
- 边界条件：翻译阶段超时，验证优化结果保留且状态为 `translate=failed`。
- 异常场景：LLM 返回非预期 JSON/空内容，验证阶段失败与错误信息落库。
- 并发场景：快速连续输入多段文本，验证旧任务结果不会覆盖新段落。
