# Prompt contract tests

该目录用于验证三类系统提示词资产的基础契约，不直接调用真实模型服务。

## 覆盖范围
- 提示词文件存在且非空
- 输出 JSON 字段声明正确
- 提示词明确要求只返回 JSON
- 质量过滤提示词占位符完整
- 测试场景 JSON 结构完整
- 质量过滤场景可完成模板渲染且不残留未替换占位符

## 文件说明
- `src-tauri/tests/prompt-scenarios.json`：三类提示词的测试场景
- `src-tauri/tests/prompt-contracts.mjs`：断言与模板渲染工具
- `src-tauri/tests/run-prompt-contract-tests.mjs`：测试入口脚本

## 运行方式
在项目根目录执行：

```bash
node src-tauri/tests/run-prompt-contract-tests.mjs
```

## 说明
- 这些测试验证的是提示词契约和场景结构，不验证真实 LLM 的语义输出质量。
- 如果后续修改了运行时代码中的字段名或占位符协议，需要同步更新提示词和测试。
