# Plan 3: LLM 客户端与任务编排

## 前置依赖
- Plan 1
- Plan 2

## 本次目标
- 封装 OpenAI 兼容 `POST /v1/chat/completions` 调用。
- 实现异步任务编排、失败降级、结果二次推送、英文后端自动复制。

## 涉及文件
- `src-tauri/src/**/llm_client.rs`
- `src-tauri/src/**/pipeline*.rs`
- `src-tauri/src/**/clipboard*.rs`
- `src-tauri/src/**/commands/*.rs`

## 详细设计
- 客户端封装：
  - 基于异步 HTTP 客户端实现 `chat/completions` 请求。
  - 模型列表通过 `/v1/models` 拉取并缓存 5 分钟。
  - 若现有能力不足，允许引入新依赖，但保持最小化。
- 异步编排：
  - 原始文本先推送，LLM 完成后再推送优化中文和英文。
  - 回填前进行 latest-only 校验，过期任务标记 `skipped`。
- 失败降级：
  - LLM 请求/解析失败时，仅保留原始文本链路，不阻断主流程。
- 英文复制：
  - 仅在最新有效结果返回英文后执行后端复制。
  - 复制失败只记录日志，不影响推送和流程状态。

## 测试案例
- 正常路径：原始先到、优化后到、英文成功复制。
- 边界条件：过期任务被丢弃且不覆盖最新结果。
- 异常场景：LLM 失败降级、复制失败容错、模型接口失败缓存回退。
