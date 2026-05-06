# Plan 1: 前端组件逻辑优化

## 前置依赖
无

## 本次目标
- 解耦 `SettingsModal` 的加载逻辑，使本地设置（get_settings）优先显示。
- LLM 模型列表（list_llm_models）改为后台异步加载。
- 优化 UI，在 LLM 标签页提供局部的加载提示。

## 涉及文件
- `src/src/components/SettingsModal.tsx`

## 详细设计
1. **状态拆分**：
   - `loading`: 仅代表核心设置（VAD/ASR/LLM Config）的加载状态。
   - `loadingModels`: 代表 LLM 模型列表的加载状态。
2. **Effect 优化**：
   - 将 `Promise.allSettled` 拆分为两个独立的异步调用。
   - `loadSettings` 完成后立即设置 `loading = false`，从而显示弹窗内容。
   - `loadModels` 在后台执行，完成后更新 `models` 列表。
3. **UI 适配**：
   - 全局 Loading 仅由 `loading` 控制。
   - 在 LLM 选项卡的模型选择下拉框中，如果 `loadingModels` 为真，显示“正在加载模型列表...”占位符。
   - 确保 `selected_model` 在模型列表中不存在时（加载中或加载失败），依然能显示在下拉框中。

## 测试案例
1. 打开设置弹窗，VAD 和 ASR 页面应立即可以操作。
2. 切换到 LLM 页面，如果模型列表还在加载，下拉框应显示加载状态。
3. 模型列表加载完成后，下拉框自动更新。
4. 如果模型列表加载失败，应有相应的 warn 日志，但不影响其他设置。
