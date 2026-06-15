# 系统设计总览（长期基线索引）

> 本文是 `docs/design/` 长期设计资产的统一入口，按模块列出「当前实际是什么样」的基线
> 设计文档。任务级「准备怎么做」的实施文档在 `docs/<日期>-<主题>/`，设计影响记录在
> `docs/adr/`。新增模块级基线文档时，必须同步更新本索引。

## 系统整体视图

StreamSpeech：Windows 瘦客户端 + GB10 服务端，做实时中文/多语种语音转写、可选 LLM 润色/翻译，
外加独立的 TTS 与音频清洗能力。架构详见仓库根 `CLAUDE.md` 与 `docs/redesign-architecture-overview.md`。

## 模块基线索引

| 模块 | 端口 | 职责 | 基线文档 | 状态 |
|---|---|---|---|---|
| 桌面客户端（Tauri） | — | mic 采集 + UI + 剪贴板 + 远端 WS | （待补） | 生效 |
| orchestrator | 8090 | 客户端 WS 终结 + SQLite + Web 管理台 + HTTP API | （待补） | 生效 |
| asr（FunASR） | 9100 / 9101 | 流式 VAD + 多模型识别 + 声纹门控 + `/transcribe` + `/embed` | `docs/asr-transcribe-api.md`（接口契约） | 生效 |
| tts（CosyVoice2） | 8095 | 零样本声音克隆 TTS | `server/tts/README.md` + `API.md` | 生效 |
| **audio-cleanup** | **8097** | **脏音频 → 干净音频（人声分离/降噪/删停顿/响度归一化）** | [`docs/design/audio-cleanup-design.md`](audio-cleanup-design.md) | **生效**（2026-06-15 GB10 构建+冒烟通过） |

> `audio-cleanup` 已实现并在 GB10 跑通：服务端基线见
> [`audio-cleanup-design.md`](audio-cleanup-design.md)，对外契约见
> [`docs/audio-cleanup-api.md`](../audio-cleanup-api.md)，实施过程见
> [`docs/2026-06-14-audio-cleanup/`](../2026-06-14-audio-cleanup/audio-cleanup.md)，决策见
> [`docs/adr/2026-06-14-audio-cleanup.md`](../adr/2026-06-14-audio-cleanup.md)。

## 跨仓关联

toolkit workspace（`github-commit-info`，部署在 GB10/G10）通过 HTTP 契约消费本系统服务：
`asr-client → FunASR /transcribe`、`toolkit-server 代理 → TTS /tts`、（规划中）
`audio-clean-client → audio-cleanup /clean`。各以契约解耦、独立部署。
