# asr-server 已退役

`server/asr-server/`（独立 sherpa-onnx + OpenAI 兼容 HTTP 转写服务）走过两段历史：

1. **2026-06 早期**：从本仓迁出 → toolkit 中台仓库（`jm-observer/toolkit` 即本地
   `D:\git\toolkit` / `D:\git\github-commit-info`）的 `crates/asr-server`，统一权威源。
2. **2026-06 中后期**：在 toolkit 短暂落地后**物理退役**——sherpa-onnx 与本仓
   FunASR 服务能力重叠，且 FunASR 在中文准确率、热词、声纹、模型丰富度上全面占优；
   GB10 上 FunASR Paraformer 反正已 GPU 常驻（实时管线服务桌面客户端），多支持一个
   离线 HTTP 端点是零边际成本。

## 当前权威源

**streaming-speech 仓自身**：`server/asr/` 的 FunASR 服务，HTTP `:9101` 同时提供：

- `POST /embed`（旧）：声纹注册时提取 embedding。
- `POST /transcribe`（**新，2026-06 加入**）：multipart 上传 wav/mp3/mp4/webm/...，
  返回 `{text, segments:[{t_start,t_end,text}], model}`。可选 form 字段 `vad`
  （默认 1 → VAD 切段含时间戳；0 → 整段一锤识别，segments 空数组）。模型由
  orchestrator 的 `asr.model` 配置控制，与流式管线共用（Paraformer / SenseVoice /
  Whisper turbo / Whisper large-v3 任选）。

容器端口映射：`server/compose.yaml` 已在 `asr` 服务上加 `127.0.0.1:9101:9101`，
**仅本机回环可达**。同机消费方（toolkit 抖音管线）走 `http://127.0.0.1:9101/transcribe`。

## toolkit 侧已完成的退役动作

- 删除 `crates/asr-server/`（含 silero_vad.onnx）。
- workspace `Cargo.toml` 移除成员、`deploy-g10.ps1` 移除 bin。
- `deploy/asr-tts/` 保留但只剩 TTS（CosyVoice2），README 改写说明 ASR 已外移。
- `crates/douyin/src/process.rs`：从 `{source:"file://...", vad}` JSON post 改成
  multipart 上传 mp4 字节；响应解析 `start/end` → `t_start/t_end`。
- 默认 `asr_url` 全部从 `:8091/.../from-source` 改为 `:9101/transcribe`，
  默认 `asr_model` 从 `sense-voice` 改为 `funasr` 兜底标签（实际模型由服务端回传）。

## 后续在哪里改

ASR 任何能力调整（新模型、热词、端点形状）都在本仓 `server/asr/app.py` 改。
toolkit 那边只是消费方，跟着 contract 走。
