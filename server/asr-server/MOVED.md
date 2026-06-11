# asr-server 已迁出本仓

`server/asr-server/`（独立 sherpa-onnx + OpenAI 兼容 HTTP 转写服务）已于 2026-06 迁出，
现归属 **toolkit 工具中台**仓库，作为统一权威来源：

- 仓库：`jm-observer/toolkit`（GitHub）/ 本地 `D:\git\toolkit`（暂仍为 `D:\git\github-commit-info`）
- 路径：`crates/asr-server`
- 部署编排：`deploy/asr-tts/`（ASR + TTS 同机 compose）
- 对外入口：toolkit-server 的 `/api/web/audio/tts` 代理（TTS），ASR 直连 `:8091`

## 为什么迁走

asr-server 当初是为 **douyin / zero 外部调用**（下载视频后转写）而建的独立服务，
**并非** streaming-speech 自身实时管线的一环——本仓的实时转写走 `server/asr`（FunASR）。
该能力的真实消费方（抖音知识管线）已统一收敛到 toolkit 中台，故 asr-server 随之迁入，
避免一份代码两处维护、反复同步漂移。

## 后续在哪里改

asr-server 的任何优化（如池化、线程数、新端点）**直接在 toolkit 仓 `crates/asr-server` 改**，
不要再在本仓恢复该目录。本仓相关引用（workspace 成员、compose profile、release 脚本）已一并移除。
