# server/audio-cleanup/ — 音频清洗服务

GB10 上的独立 HTTP 服务：脏音频 → 干净音频（人声分离 / 降噪去混响 / 删停顿 / 响度归一化）。
**与生产 `asr`/`orchestrator` 完全隔离**（独立 compose、独立镜像、独立端口 `8097`）。

> 设计/契约：[`docs/2026-06-14-audio-cleanup/`](../../docs/2026-06-14-audio-cleanup/audio-cleanup.md)
> ｜ ADR：[`docs/adr/2026-06-14-audio-cleanup.md`](../../docs/adr/2026-06-14-audio-cleanup.md)
> ｜ 调用方 API 手册：`docs/audio-cleanup-api.md`（M3 补）

## 拓扑

| 服务 | 容器 | 宿主端口 | 引擎 |
|---|---|---|---|
| audio-cleanup | `audio-cleanup-1` | **127.0.0.1:8097** | ffmpeg + Demucs(htdemucs) + DeepFilterNet + silero-vad + pyloudnorm |

## 宿主机资产（持久化，不进镜像）

| 路径 | 内容 | 来源 |
|---|---|---|
| `~/audio-cleanup/torch-cache/` | demucs/DF 权重缓存 | 首次运行下载（GitHub 不稳则手动放 checkpoints） |

> silero VAD 模型随 `silero-vad` pip wheel 内置，无需单独挂载/下载。

```bash
mkdir -p ~/audio-cleanup/torch-cache
```

## 部署

```bash
# GB10 ~/server/audio-cleanup（本仓 server/audio-cleanup/ scp 而来）
cd ~/server/audio-cleanup
docker compose -f compose.cleanup.yaml up -d --build
docker compose -f compose.cleanup.yaml logs --tail=20 audio-cleanup
```

`restart: unless-stopped`，GB10 重启自动拉回。镜像构建 ~10-15 分钟（arm64+CUDA13）。

### 代码改动后重建

```bash
scp server/audio-cleanup/{app.py,pipeline.py} fengqi@192.168.0.68:~/server/audio-cleanup/
ssh fengqi@192.168.0.68 'cd ~/server/audio-cleanup
  docker compose -f compose.cleanup.yaml build audio-cleanup
  docker compose -f compose.cleanup.yaml up -d audio-cleanup'
```

## 冒烟

```bash
# 健康
curl -s http://127.0.0.1:8097/health      # {model_loaded, stages_available, gpu}

# 给人听（降噪 + 压低停顿，默认 48k）
curl -sS -F audio=@noisy.wav -F denoise=1 -F pause=duck \
  http://127.0.0.1:8097/clean -o out.wav

# 带 BGM 视频 → 去乐人声给 ASR（separate=1，删停顿关，16k）
curl -sS -F audio=@bgm.mp4 -F separate=1 -F pause=off -F sr=16000 \
  http://127.0.0.1:8097/clean -o vocals.wav

# 断言输出可读、时长 > 0
ffprobe -v error -show_entries format=duration -of csv=p=0 out.wav

# torch 版本运行期校验（与构建期断言一致；torchaudio 仅打印）
docker compose -f compose.cleanup.yaml exec audio-cleanup python3 -c \
  "import torch,torchaudio; v=torch.__version__; assert v.startswith('2.13') and 'cu130' in v, v; print('torch',v,'torchaudio',torchaudio.__version__)"
```

## 测试

```bash
# 纯逻辑（任意机）
python server/audio-cleanup/test_pipeline.py

# aiohttp 行为（缺 audio→400 / 超时长→422 / 超时→504 / 队列满→503 / 成功→200+头+临时目录清理+计数不泄漏）
# 需 aiohttp，在容器里跑：
CID=$(docker compose -f compose.cleanup.yaml ps -q audio-cleanup)
docker cp server/audio-cleanup/test_app.py $CID:/app/ && docker exec $CID python3 /app/test_app.py
```

## 请求参数（`POST /clean` multipart）

| 字段 | 默认 | 说明 |
|---|---|---|
| `audio` | 必填 | 任意 ffmpeg 可解码音/视频 |
| `separate` | `0` | `1` 开人声分离（Demucs，慢） |
| `denoise` | `1` | DeepFilterNet 降噪+去混响（固定 48k 内部处理） |
| `pause` | `duck` | `drop` 删停顿 / `duck` 压低 / `off` 不动 |
| `level` | `balanced` | `gentle`/`balanced`/`aggressive`（降噪强度） |
| `loudness` | `-16` | 目标 LUFS；`off` 关 |
| `sr` | `48000` | 输出采样率（**仅末端生效**，不影响 DF 的 48k） |
| `format` | `wav` | `wav`/`mp3`/`flac` |

错误体统一 `{"error": "..."}`：400 解析/解码失败、413 超 `CLIENT_MAX_SIZE`、422 超
`MAX_DURATION_SEC`、503 队列满、504 超 `PROCESS_TIMEOUT_SEC`、500 内部异常。

## GB10 构建坑（同 server/tts）

- 基于 `funasr-asr:arm64`，**不重装 torch/torchaudio**（dev 版才支持 sm_121）。
- Dockerfile 有 torch 版本**构建期断言**，挡 demucs/DF 偷偷升级 torch。
- pip 走 Aliyun 镜像 + `--no-cache-dir`。
- htdemucs/DF 权重走 GitHub release，GB10 直连不稳——超时则手动放进 `torch-cache/hub/checkpoints/`。
