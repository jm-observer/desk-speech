# prompt-system-tests 设计总览

## 时间
- 创建日期：2026-05-13
- 最后更新：2026-05-13

## 项目现状
- 当前系统已经存在三类与 LLM 后处理直接相关的提示词配置：`optimize_prompt_template`、`translate_prompt_template`、`llm_prompt_template`。
- 文本优化和翻译的前端类型定义位于 `src/src/api/tauri-client.ts:48`，配置界面位于 `src/src/components/SettingsModal.tsx:146`。
- 质量过滤配置的前端类型定义位于 `src/src/api/tauri-client.ts:100`，前端编辑入口位于 `src/src/components/SettingsModal.tsx:245`。
- 后端默认提示词和解析协议位于 `src-tauri/src/llm_settings.rs:13` 与 `src-tauri/src/llm_client.rs:75`。
- 质量过滤模板渲染和占位符协议位于 `src-tauri/src/config/quality_filter.rs:112`。
- 当前仓库缺少独立的提示词文档资产，也缺少用于验证提示词契约、占位符和测试场景结构的根目录测试程序。

## 整体目标
- 在 `docs` 目录下沉淀三份可独立审阅、可复用的系统提示词文件。
- 在项目根目录新增 `tests` 目录，保存三类提示词的测试场景与测试程序。
- 通过离线契约测试验证提示词输出字段、占位符和场景结构，避免破坏现有解析链路。
- 保持本轮改动聚焦，不改动现有应用逻辑，不引入额外测试框架。

## 范围与非目标
### 范围内
- 新增“程序表达与优化”“中文翻译”“质量过滤”三份系统提示词文档。
- 新增详细设计文档。
- 新增根目录 `tests` 目录及离线测试程序。
- 新增三类提示词测试场景 JSON。

### 非目标
- 不修改运行时默认提示词加载方式。
- 不将 docs 下的提示词文件直接接入后端运行链路。
- 不新增 Vitest、Jest、Playwright 等测试框架。
- 不调用真实 LLM 做自动语义评测。

## 核心设计决策
### 决策 1：沿用现有 docs 日期目录约定
新建 `docs/2026-05-13-prompt-system-tests/`，与现有 `docs/2026-05-05-final-discard-filter/` 等目录风格保持一致，便于归档和审阅。

### 决策 2：三份提示词严格对齐现有解析协议
- 文本优化提示词输出字段固定为 `text_optimized`，与 `src-tauri/src/llm_client.rs:142` 对齐。
- 翻译提示词输出字段固定为 `text_english`，与 `src-tauri/src/llm_client.rs:156` 对齐。
- 质量过滤提示词输出字段固定为 `decision`、`confidence`、`reason`，与 `src-tauri/src/llm_client.rs:285` 对齐。

### 决策 3：质量过滤提示词使用双花括号占位符
质量过滤配置当前支持 `{{text_raw}}`、`{{text_optimized}}`、`{{text_english}}`，由 `src-tauri/src/config/quality_filter.rs:119` 负责替换。本轮文档和测试将严格使用这套占位符协议。

### 决策 4：测试采用 Node 原生离线契约测试
根目录 `package.json:6` 当前没有测试基础设施，因此优先使用 Node 原生脚本完成存在性、字段声明、占位符和场景结构校验，避免额外依赖。

### 决策 5：场景测试使用“契约期望”而非固定模型输出
LLM 输出天然存在波动，本轮不使用全文精确断言，而是验证：
- 提示词是否声明了正确 JSON 字段；
- 是否要求只返回 JSON；
- 是否保留关键技术标识；
- 质量过滤场景是否能完成模板渲染且不残留占位符。

## 文件设计
### 新增文档文件
- `docs/2026-05-13-prompt-system-tests/program-expression-optimization.system.md`
- `docs/2026-05-13-prompt-system-tests/chinese-translation.system.md`
- `docs/2026-05-13-prompt-system-tests/quality-filter.system.md`
- `docs/2026-05-13-prompt-system-tests/prompt-system-tests.md`

### 新增测试文件
- `tests/README.md`
- `tests/prompt-scenarios.json`
- `tests/prompt-contracts.mjs`
- `tests/run-prompt-contract-tests.mjs`

## 三类提示词协议说明
### 1. 程序表达与优化
用途：对 ASR 原始中文文本做纠错、去噪、补标点和工程表达整理。

输入：原始中文文本。

输出契约：
```json
{"text_optimized":"优化后的中文文本"}
```

约束：
- 只返回 JSON。
- 不返回 Markdown。
- 不附加解释。
- 保留代码、配置项、路径、命令、函数名、变量名等工程标识。
- 保持原意，不扩写。

### 2. 中文翻译
用途：将优化后的中文文本翻译为英文。

输入：优化后的中文文本。

输出契约：
```json
{"text_english":"English translation"}
```

约束：
- 只返回 JSON。
- 不返回 Markdown。
- 不附加解释。
- 保留代码、路径、命令、URL、类名、函数名、模型名等技术标识。
- 忠实翻译，不添加原文没有的信息。

### 3. 质量过滤
用途：对最终文本进行保留/丢弃判定。

输入模板：
- `{{text_raw}}`
- `{{text_optimized}}`
- `{{text_english}}`

输出契约：
```json
{"decision":"KEEP","confidence":0.9,"reason":"包含明确语义，应保留"}
```

约束：
- `decision` 只能是 `KEEP` 或 `DISCARD`。
- `confidence` 必须是 0 到 1 之间的数值。
- `reason` 必须是简短中文说明。
- 不确定时优先 `KEEP`。
- 仅在明显低信息量、噪音、填充词、孤立称呼或高重复无语义内容时建议 `DISCARD`。

## 测试设计
### 测试目标
- 验证三份提示词文件存在且非空。
- 验证三份提示词声明了正确的输出 JSON 字段。
- 验证提示词显式要求“只返回 JSON”。
- 验证质量过滤提示词包含全部必需占位符。
- 验证 `prompt-scenarios.json` 结构完整。
- 验证质量过滤场景可正确渲染模板，且渲染后无残留占位符。

### 测试程序结构
#### `tests/prompt-contracts.mjs`
封装纯函数工具：
- 读取文本文件
- 断言关键词存在
- 校验场景数组结构
- 渲染质量过滤模板
- 检查渲染结果中是否残留 `{{...}}`

#### `tests/run-prompt-contract-tests.mjs`
作为测试入口，执行所有断言并输出 PASS/FAIL 结果，失败时返回非零退出码。

## 测试场景矩阵
### 程序表达与优化
- 去除口语噪音：`嗯这个我们明天下午三点和客户开会`
- 程序表达整理：`把setting modal里面质量过滤的保存逻辑优化一下`
- 保留接口/函数名：`先调用getSettings然后再saveQualityFilterConfig`
- 处理口语化报错描述：`啊啊这个就是报错了`
- 保留命令和技术名词：`vite build失败因为typescript类型不匹配`

### 中文翻译
- 保留组件名：`请优化 SettingsModal 里的质量过滤配置保存逻辑。`
- 保留命令：`运行 npm run build 验证类型检查和打包。`
- 保留文件路径：`不要修改 src-tauri/src/settings.rs 的数据库键名。`
- 翻译质量过滤表述：`这个分段应该被质量过滤器丢弃。`
- 翻译工程沟通表达：`把程序表达优化为更适合工程沟通的中文。`

### 质量过滤
- 丢弃填充词：`嗯`
- 丢弃短促回应：`ok`
- 丢弃孤立姓名：`张三`
- 丢弃高重复内容：`啊啊啊啊`
- 保留明确会议信息：`明天下午三点和客户开会`
- 保留明确技术任务：`把状态机拆成三个阶段实现`
- 丢弃低信息重复回应：`好的好的好的`
- 保留明确修复任务：`修复 translate_prompt_template 的 JSON 字段解析问题`

## 实施步骤
1. 在 `docs/2026-05-13-prompt-system-tests/` 下创建三份系统提示词文档。
2. 在同目录下写入设计文档 `prompt-system-tests.md`。
3. 在项目根目录创建 `tests/`。
4. 编写 `prompt-scenarios.json` 保存三类测试样例。
5. 编写 `prompt-contracts.mjs` 提供断言与渲染函数。
6. 编写 `run-prompt-contract-tests.mjs` 执行测试。
7. 运行测试并确认全部通过。

## 验证策略
1. 运行：`node tests/run-prompt-contract-tests.mjs`
2. 如需进一步确认无连带影响，可运行：`npm --prefix src run build`
3. 如需进一步确认后端原有逻辑未受影响，可运行：`cargo test --manifest-path src-tauri/Cargo.toml`

## 风险与待定项
- 风险 1：离线契约测试无法验证真实模型输出质量，只能验证协议和样例结构。
- 风险 2：如果后续运行时代码修改了字段名或占位符，本轮测试也需要同步更新。
- 风险 3：当前“中文翻译”在系统真实链路中是“中文转英文”，若以后扩展为多语种翻译，需要重新定义输出契约。

## 后续扩展
- 可在后续单独增加“手动评测脚本”，对接真实模型服务，对样例做半自动语义检查。
- 可在后续把提示词资产迁移到更适合作为运行时资源的目录，再设计默认值加载机制。
