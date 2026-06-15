# ADR 2026-06-14：新增音频清洗服务（audio-cleanup）

## 背景

录音设备不专业 + 环境噪声（呼吸、翻页、桌椅、BGM）导致：① 给人听的录音质量差；
② TTS 声音克隆参考音脏、克隆质量差；③ 带 BGM 的抖音视频 ASR 识别率低。仓库现状无任何
降噪 / 人声分离 / 语音增强能力（团队早期明确放弃「烂素材 + 后处理」，转而用干净素材库），
现有 `server/asr` 的 VAD 只切片定位、不输出清洗后音频。需要一个能「脏音频 → 干净音频文件」
的独立能力。

## 决策

在 GB10 新增**独立隔离**的音频清洗 HTTP 服务 `audio-cleanup`（端口 `8097`，独立
`compose.cleanup.yaml`），管线为 `ffmpeg 解码 → Demucs 人声分离 → DeepFilterNet 降噪去混响
→ silero-vad 删停顿 → pyloudnorm 响度归一化 → 编码`。以 toolkit `asr-client` 为样板新增
`audio-clean-client` crate，供 douyin 管线、toolkit-server 代理、zero-desktop 三类消费方接入。

关键决策点：

- **Demucs 进 v1**：人声分离对带 BGM 视频的 ASR 识别率是实打实增益，不只是 TTS 素材附属；
  随之 Demucs × GB10 `torch 2.13.dev+cu130` 兼容性升为 v1 阻塞项，单列 Plan 1 spike 先验证
  （GPU 不通则 CPU 回退）。
- **DeepFilterNet 固定 48k**：`deep-filter` 仅支持 48kHz；管线契约规定 DF 内部固定 48k mono，
  请求的 `sr` 只在末端重采样输出，禁止 DF 前按 `sr` 降采样。
- **同步 + 限额**：v1 走同步，固化 `CLIENT_MAX_SIZE`/`MAX_DURATION_SEC`/`QUEUE_MAX`/
  `PROCESS_TIMEOUT_SEC` 与 413/422/503/504 `{"error":...}` 错误体。
- **`restart: unless-stopped` + 确定的模型生命周期**：被 douyin 自动管线消费需自愈，与 TTS 一致；
  为避免与 asr/tts/vLLM 抢显存，**必须**二选一——方案 A（Demucs 固定 CPU，GPU 零占用，默认取向）
  或方案 B（GPU 懒加载 + 空闲 `IDLE_UNLOAD_SEC` 后卸载 + `empty_cache`）；**禁止载入后无限常驻 GPU**。
  由 Plan 1 结论拍板。
- **base 契约统一**：客户端字段 `clean_base_url`（不含 `/clean`），内部固定拼 `/clean`；
  toolkit-server 的 `CLEAN_BASE_URL` env 无默认，缺失即 503、上游不可达 502。
- **端口暴露最小**：默认仅 `127.0.0.1:8097`；zero-desktop 走 toolkit-server :8788 代理，
  不把 :8097 暴露到 LAN。

## 影响范围

- **本仓**：新增 `server/audio-cleanup/`、`docs/audio-cleanup-api.md`（契约）、
  `docs/design/audio-cleanup-design.md`（基线，实现后回写）；不改 asr/orchestrator/tts/生产 compose。
- **toolkit 仓**：新增 `crates/audio-clean-client/`；改 `crates/douyin/src/process.rs` + `Job` +
  CLI；toolkit-server 加 `/api/web/audio/clean` 路由 + `CLEAN_BASE_URL` env；`CLAUDE.md` 服务清单。
- **运行行为**：GB10 多一个常驻（懒加载）容器，占用端口 8097。

## 取舍

- **降噪选 DeepFilterNet 而非谱减/noisereduce**：谱减易削掉齿音/气声变闷糊；DF 保真更好、
  自带 dereverb、CPU 可实时。放弃 `resemble-enhance` 作默认（会「美化」改音色，污染 TTS 素材），
  仅 `aggressive` 档显式启用。
- **人声分离选 Demucs htdemucs 而非 UVR5**：UVR5 偏 GUI、无头部署麻烦。备选 MDX-Net/spleeter
  仅在 Demucs GB10 不兼容时启用。
- **VAD 自带 silero 而非内网调 FunASR FSMN**：避免跨服务耦合，保持本服务独立。
- **同步而非异步任务**：v1 简单优先（同步 + 可终止子进程 + 限额）；大文件超时频发再引入
  `/clean/async` 任务模式（后续可选增强，不在本次 Plan 1–4 范围内）。
- **独立 compose 而非并入生产栈**：清洗是离线低频任务，不该与实时识别共享生命周期。

## 文档同步

- 实现 M1 后**新增** `docs/design/audio-cleanup-design.md`（长期基线，描述落地后的实际结构），
  并在 `docs/design/system-overview.md` 索引中将本模块由「规划中」改为「生效」。
- M3 写 `docs/audio-cleanup-api.md`（对外 HTTP 契约手册）。
- toolkit 仓 `CLAUDE.md` 服务清单补登 `:8097`。

## 实施记录（2026-06-15 GB10 落地）

落地时遇到 arm64 特有的构建/兼容问题，均已解决（详见 `docs/design/audio-cleanup-design.md`）：

- **DeepFilterLib（DF 的 Rust DSP 后端）arm64 无预编译 wheel** → Dockerfile 加 build-essential
  + rustup（rsproxy.cn）现编。
- **DF 0.5.6 import 已删的 `torchaudio.backend.common`** → `pipeline.py` 加兼容 shim。
- **DF init_logger 依赖 `git`** → 镜像装 git。
- **Plan 1 spike**：Demucs cuda 可用、gpu_peak 0.91GB → 定 `CLEAN_DEMUCS_DEVICE=cuda`。
- code review 修复 3 个资源/正确性问题：临时目录 finally 清理、超时按**进程组** kill（含 ffmpeg
  孙进程）、等待计数 try/finally 防取消泄漏；`test_app.py` 覆盖。

## 关联项

- 总览：[`docs/2026-06-14-audio-cleanup/audio-cleanup.md`](../2026-06-14-audio-cleanup/audio-cleanup.md)
- Plan 1（Demucs 风险 spike）/ Plan 2（服务+契约）/ Plan 3（toolkit client+douyin+代理）/
  Plan 4（zero-desktop 入口）：同目录
- PR / commit：待实施后回填
