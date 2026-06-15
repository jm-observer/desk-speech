# audio-cleanup 服务设计基线（长期资产）

> 描述**当前实际实现**的 audio-cleanup 服务（GB10 :8097）。任务级实施过程见
> `docs/2026-06-14-audio-cleanup/`，决策见 `docs/adr/2026-06-14-audio-cleanup.md`，
> 对外契约见 `docs/audio-cleanup-api.md`。状态：**生效**（2026-06-15 GB10 构建+冒烟通过）。

## 职责与边界

脏音频 → 干净音频的离线 HTTP 服务。**输入整段文件 / 输出整段文件**，不做实时流式，不替代
ASR/TTS（输出可再喂它们）。与生产 `asr`/`orchestrator` 完全隔离（独立 compose/镜像/端口）。

## 部署形态

| 项 | 值 |
|---|---|
| 端口 | `127.0.0.1:8097`（仅 loopback，桌面端经 toolkit-server 代理） |
| 编排 | `server/audio-cleanup/compose.cleanup.yaml`，`restart: unless-stopped`，`runtime: nvidia` |
| 镜像 | `audio-cleanup:gb10`，基于 `funasr-asr:arm64`（torch 2.13.dev+cu130 / sm_121） |
| 权重 | 宿主 `~/audio-cleanup/torch-cache`（Demucs htdemucs）；DeepFilterNet3 随 wheel 内置；silero 随 wheel 内置 |

## 处理管线（`pipeline.py`）

固定顺序，每 stage 按请求开关：

```
ffmpeg 解码(→48k) → Demucs htdemucs 取 vocals(separate) → DeepFilterNet 降噪+去混响(denoise)
→ silero-vad 删停顿(pause=drop|duck|off) → pyloudnorm 归一化(loudness) → ffmpeg 编码(按 sr/format)
```

- **采样率铁律**：DeepFilterNet 仅支持 48kHz。DF stage 固定在 48k mono 上工作；请求 `sr`
  只在末端 encode 重采样，绝不在 DF 前降采样。
- **device**：Demucs/DF 跑 **cuda**（`CLEAN_DEMUCS_DEVICE=cuda`，Plan 1 实测 gpu_peak 0.91GB、
  净算约 cpu 的 4×）；模型在子进程内加载、随子进程退出释放，空闲 GPU 零占用。

## 服务层（`app.py`，aiohttp）

| 关注点 | 实现 |
|---|---|
| 并发 | 全局 `Semaphore(1)` 串行化整条 pipeline；`_waiting` 计数等待请求，超 `QUEUE_MAX`(4) 立即 503 |
| 等待计数防泄漏 | `_waiting += 1` 后 `await acquire()` 包在 try/finally，等待期被取消也会减回（否则永久错误 503） |
| 可终止执行 | pipeline 跑在 `start_new_session=True` 的子进程；超时 `PROCESS_TIMEOUT_SEC`(600) 按**进程组** SIGKILL（含 ffmpeg 孙进程），真实回收后才释放锁、返回 504 |
| 临时目录 | 每请求 `mkdtemp`，handler 外层 try/finally `rmtree`（body 已读入内存后删，无泄漏） |
| 限额 | `CLIENT_MAX_SIZE`(512MiB)→413、`MAX_DURATION_SEC`(600)→422、错误体统一 `{"error":...}` |
| 端点 | `POST /clean`（multipart in / 二进制音频 out + `X-Cleanup-*` 头）、`GET /health` |

## GB10 arm64 构建要点（与 server/tts 同源 + 本服务特有）

- 基于 `funasr-asr:arm64`，**不重装 torch/torchaudio**；Dockerfile 有 torch 2.13+cu130 **构建期断言**。
- **DeepFilterLib（DF 的 Rust DSP 后端）arm64 无预编译 wheel**：Dockerfile 装 `build-essential`
  + rustup（rsproxy.cn 镜像）+ cargo 源替换，现编 DeepFilterLib。
- **torchaudio shim**：DF 0.5.6 的 `df.io` 仍 import 已被删的 `torchaudio.backend.common.AudioMetaData`，
  `pipeline.py` 在 import df 前补兼容 shim。
- **git**：DF 的 init_logger 取自身 commit 需 `git`，否则崩；镜像装了 git。
- pip 走 Aliyun 镜像 + `--no-cache-dir`。

## 测试

- `test_pipeline.py`：CleanOpts 纯逻辑（任意机可跑）。
- `test_app.py`：aiohttp 行为（缺 audio→400、超时长→422、超时→504、队列满→503、成功→200+头+
  临时目录清理+等待计数不泄漏）。需 aiohttp，在容器/CI 跑：`docker exec <cid> python3 /app/test_app.py`。
- stage/端到端在 GB10 冒烟（README）。

## 消费方（toolkit 仓）

`audio-clean-client` crate → douyin 前置清洗 / toolkit-server `/api/web/audio/clean` 代理 /
zero-desktop `speech_clean_recording`。契约以 `docs/audio-cleanup-api.md` 为准。
