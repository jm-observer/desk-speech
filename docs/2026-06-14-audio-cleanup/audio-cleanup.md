# 音频清洗服务（audio-cleanup）— 总览

| 项 | 值 |
|---|---|
| 创建 / 更新 | 2026-06-14 / 2026-06-14 |
| 状态 | 设计草案（待评审，未实现） |
| 任务目录 | `docs/2026-06-14-audio-cleanup/` |
| 设计影响记录 | [`docs/adr/2026-06-14-audio-cleanup.md`](../adr/2026-06-14-audio-cleanup.md) |
| 长期基线（实现后回写） | `docs/design/audio-cleanup-design.md`（M1 完成后创建，见 ADR「文档同步」） |

## 项目现状

仓库现状（依据 `docs/asr-transcribe-api.md`、`server/tts/STATUS.md` 与代码）：

- `server/asr` 已有 **FSMN-VAD 切句**和 **ffmpeg 格式归一**，但 VAD 只做「切片定位」，
  **不输出降噪后的整段音频**；
- 全仓**没有任何**降噪 / 人声分离 / 语音增强依赖（`noisereduce` / `demucs` /
  `deepfilternet` / `resemble-enhance` 一个都没有）；团队早期明确放弃「烂素材 + 后处理」
  路线（`server/tts/STATUS.md`：个人录音 `me.wav` SNR 37dB 被弃用）。

因此「想要一段清洗后的干净音频文件本身」这一诉求，现有服务无法满足，需新建独立服务。

跨仓现状（`D:\git\github-commit-info` toolkit workspace，部署在 GB10/G10）：

- `crates/asr-client` 已是「音频文件 → multipart 上传 FunASR `/transcribe` → 强类型响应」
  的成熟样板；
- `crates/douyin` 抖音管线（`process.rs::process_one`）下载 mp4 后经 asr-client 转写，
  视频普遍带 BGM/音效，是 ASR 识别的最大干扰源；
- `zero-desktop`（本地 Windows）Speech 模块做本地录音、经 `toolkit-server :8788` 上传，
  现走 `/api/web/audio/tts` 代理消费 TTS（:8095），是已有的「桌面端不直连 GB10、走代理」
  先例。

## 整体目标

在 GB10 上新增一个**独立的音频清洗 HTTP 服务**（端口 `8097`），对一段「脏」录音做
**人声分离 / 降噪去混响 / 删停顿 / 响度归一化**，输出「干净」音频文件；并以 toolkit 的
`asr-client` 为样板，让 douyin 管线、zero-desktop、TTS 素材准备三类消费方以**统一 HTTP
契约**接入。与生产 `asr` / `orchestrator` **完全隔离**，编排模式照搬 `server/tts`。

### 目标用途（按优先级）

| 优先级 | 用途 | 主要诉求 | 链路开关默认 |
|---|---|---|---|
| **P0** | 给人听（考试录音、会议回放去呼吸/翻页/底噪） | 听感干净、不损语音清晰度 | 降噪 ✅ / 删停顿 ✅ / 人声分离 ❌ |
| **P1** | TTS 克隆素材（动画角色、参考音清洗） | 人声纯净、去 BGM/混响、音色不被「美化」改 | 人声分离 ✅ / 去混响 ✅ / 降噪温和 |
| **P2** | ASR 预处理（带 BGM 视频 / 顽固底噪） | 剥 BGM 提升识别率、宁留底噪别削辅音 | 人声分离 ✅（视频）/ 降噪温和 ✅ |

> 三类用途共用同一条处理链，靠请求参数（开关 + 激进度档位）区分，不为每个用途单独建端点。

### 架构拓扑

```
                         GB10 (192.168.0.68, arm64+CUDA13, Docker)
  调用方  ──HTTP──►  audio-cleanup  :8097   ← 本服务（新增，独立 compose）
 (本机/LAN)         ├─ ffmpeg          解码/编码
                    ├─ Demucs(htdemucs) 人声分离（separate=1）
                    ├─ DeepFilterNet   降噪 + 去混响（DF 固定 48k 内部处理）
                    ├─ silero-vad      删停顿（复用客户端那份 .onnx）
                    └─ pyloudnorm      响度归一化（EBU R128）

 联动（toolkit 仓，跨仓以 §HTTP 契约对接）：
  douyin(G10同机) ──直连 127.0.0.1:8097──► /clean ──► 干净wav ──► :9101 /transcribe
  zero-desktop(本地Win) ──► toolkit-server :8788 /api/web/audio/clean ──代理──► :8097
```

### 仓库落点

| 路径 | 内容 | 归属仓 |
|---|---|---|
| `server/audio-cleanup/{app.py,pipeline.py,Dockerfile,compose.cleanup.yaml,README.md}` | 服务端实现 + 运维 | 本仓 |
| `docs/audio-cleanup-api.md` | 调用方 HTTP 契约手册（M3 写） | 本仓 |
| `docs/design/audio-cleanup-design.md` | 长期基线（M1 后回写） | 本仓 |
| `crates/audio-clean-client/`（Plan 3） | Rust 客户端（照抄 asr-client） | **toolkit 仓** |
| `crates/douyin/src/process.rs` 接入 + toolkit-server 代理路由（Plan 3） | 服务端调用方改动 | **toolkit 仓** |
| zero-desktop `speech_clean_recording` command（Plan 4，可后置） | 桌面端入口 | **toolkit 仓** |

## Plan 拆分

| Plan | 标题 | 职责 | 依赖 | 执行顺序 | 状态 |
|---|---|---|---|---|---|
| [Plan 1](audio-cleanup-plan-1.md) | Demucs GB10 风险验证（spike） | 先证明 htdemucs 在 GB10（GPU→CPU 回退）能跑通，定 device 策略与 `MAX_DURATION_SEC` | 无 | 1（最先） | ✅ 已完成（2026-06-15 GB10 实跑：cuda+cpu 均 OK，gpu_peak 0.91GB，**拍板 cuda**；结论见 spike README） |
| [Plan 2](audio-cleanup-plan-2.md) | 清洗服务 + HTTP 契约 | 建 `server/audio-cleanup`：管线 + 并发控制 + 模型生命周期 + `/clean`/`/health` + 限额错误体 | Plan 1 | 2 | ✅ 已完成（2026-06-15 GB10 构建+冒烟通过；`test_app.py` 5/5 验证 503/504/临时目录清理/计数不泄漏） |
| [Plan 3](audio-cleanup-plan-3.md) | toolkit 联动（client+douyin+代理） | toolkit 仓加 `audio-clean-client`、douyin 前置清洗、toolkit-server 代理 | Plan 2 | 3 | 已完成（toolkit 仓 clippy/fmt/test 全过；契约手册 `docs/audio-cleanup-api.md` 已补） |
| [Plan 4](audio-cleanup-plan-4.md) | zero-desktop 桌面端入口 | zero-desktop Speech 模块经 toolkit-server 代理清洗本地录音 | Plan 3 | 4（可后置） | 后端命令已完成（`speech_clean_recording`，clippy/fmt/test 过）；前端按钮 + toolkit-server URL 来源待接 |

> Plan 1 是**解风险前置**：Demucs × GB10 `torch 2.13.dev+cu130` 兼容性是本设计最大不确定性，
> 必须先验证再投入服务骨架。Plan 2/3/4 在契约层解耦：Plan 3/4 全部改动落在 **toolkit 仓**，
> 仅依赖 Plan 2 产出的 HTTP 契约。**zero-desktop（Tauri 桌面侧）单列 Plan 4 且可后置**——不让
> 桌面端改动阻塞服务端联动（Plan 3）的评审与交付。

## 风险与待定项

| 项 | 取向 / 状态 |
|---|---|
| **Demucs × GB10 兼容性** | ✅ **已解**（2026-06-15 Plan 1 spike）：cuda + cpu 均跑通，**拍板 cuda**（gpu_peak 0.91GB、净算约 cpu 的 4×、子进程退出即释放）。torch 仍 2.13+cu130（demucs 未升级）。降级排查可切 cpu |
| **DeepFilterNet 采样率约束** | `deep-filter` 当前仅支持 **48kHz**。管线契约固定：DF stage 内部转 48k mono 处理，**最后一步**才按请求 `sr` 重采样输出。详见 Plan 2 §管线 |
| **同步长任务** | v1 同步 + 排队；固化 `client_max_size` / `max_duration_sec` / 排队上限（503 busy）/ 504 超时 JSON 错误体。Demucs CPU 处理时长可达音频时长 ~1.5×，长视频易撞超时，故时长上限随 GPU/CPU 模式调。详见 Plan 2 |
| **删停顿默认** | 默认 `duck`（压低不删）：删段会改时长、影响考试录音对题；ASR/douyin 路径固定 `off`。**待用户确认** |
| **采样率默认** | 给人听默认 48k 文件偏大；ASR/douyin 固定 16k 不受影响。是否给人听也降到 24k？**待用户确认** |
| **部署 restart 策略 + 模型生命周期** | 已拍板 `restart: unless-stopped`（被 douyin 自动管线消费，需自愈）。空闲显存靠确定的生命周期：方案 A（Demucs 固定 CPU，默认）或 B（GPU 懒加载 + idle TTL 卸载 + `empty_cache`），**禁止无限常驻 GPU**，由 Plan 1 拍板。详见 Plan 2 §模型生命周期 |
| **批量目录模式** | v1 只做 HTTP 单文件（toolkit 逐文件调）；本地 CLI 批处理作为后续可选增强（需要时单列 Plan），不在本次 Plan 1–4 范围 |
