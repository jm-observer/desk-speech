# audio-cleanup `/clean` HTTP API（音频清洗）

> 权威源。toolkit 仓 `audio-clean-client` / 其他消费方对接前以本文为准。
>
> 实现：`server/audio-cleanup/app.py`（aiohttp）+ `pipeline.py`（清洗管线）。
> 设计：`docs/2026-06-14-audio-cleanup/`，决策：`docs/adr/2026-06-14-audio-cleanup.md`。

## 概览

| 项 | 值 |
|---|---|
| 方法 | `POST` |
| 路径 | `/clean` |
| 同机 base | `http://127.0.0.1:8097` |
| Content-Type（请求） | `multipart/form-data` |
| Content-Type（响应） | `audio/wav`（或按 `format`）；元数据在响应头 |
| 最大上传 | 512 MiB（`CLEAN_CLIENT_MAX_SIZE`） |
| 最大时长 | 600 s（`CLEAN_MAX_DURATION_SEC`；CPU 模式 Demucs 可下调） |
| 鉴权 | 无。仅监听 `127.0.0.1:8097`，不对 LAN 暴露。桌面端经 toolkit-server `:8788` 代理。 |
| 并发 | 单 worker 串行（Semaphore(1) 包住整条 pipeline）；等待队列 `CLEAN_QUEUE_MAX=4`，满则 503。 |

## 请求

`multipart/form-data` 字段：

| 字段 | 必填 | 类型 | 默认 | 说明 |
|---|---|---|---|---|
| `audio` | ✅ | file | — | 任意 ffmpeg 可解码音/视频（wav/mp3/mp4/m4a/webm/flac/…）；视频自动抽音轨。 |
| `separate` | ❌ | str | `0` | `1`/`true`/`on`/`yes` → Demucs 人声分离取 vocals（去 BGM；慢）。 |
| `denoise` | ❌ | str | `1` | DeepFilterNet 降噪+去混响。**内部固定 48k 处理**。 |
| `pause` | ❌ | str | `duck` | `drop`=删非语音段（改时长）/ `duck`=压低保节奏 / `off`=不动。 |
| `level` | ❌ | str | `balanced` | `gentle`（宁留底噪）/ `balanced` / `aggressive`。降噪强度。 |
| `loudness` | ❌ | str | `-16` | 目标 LUFS（EBU R128 归一化）；`off` 关闭。 |
| `sr` | ❌ | int | `48000` | 输出采样率（`16000`/`24000`/`48000`）。**仅末端生效**，不影响 DF 的 48k。 |
| `format` | ❌ | str | `wav` | `wav`/`mp3`/`flac`。 |

> **采样率契约**：DeepFilterNet 仅支持 48kHz。服务端在 DF stage 内部固定 48k mono 处理，
> 请求的 `sr` 只在最后编码一步重采样输出——绝不在 DF 前按 `sr` 降采样。

## 响应

成功 `200 OK`：响应体为二进制音频（`Content-Type` 按 `format`）。清洗元数据在响应头：

| 头 | 示例 | 说明 |
|---|---|---|
| `X-Cleanup-Stages` | `decode,separate,denoise,vad-duck,loudness,encode` | 实际执行的 stage 序列。 |
| `X-Cleanup-In-LUFS` | `-28.3` | 输入响度（归一化前）。 |
| `X-Cleanup-Out-LUFS` | `-16.0` | 输出响度。 |

## 错误

非 2xx 一律 `{"error": "<message>"}`：

| 状态 | 触发 | 处理建议 |
|---|---|---|
| `400` | 缺 `audio` / 非 multipart / 解析失败 / `decode failed` | 检查字段名是 `audio`；排查原文件。 |
| `413` | 上传超 `CLIENT_MAX_SIZE` | 先转码/截取。 |
| `422` | 音频时长超 `MAX_DURATION_SEC` | 切分后再传。 |
| `503` | 等待队列超 `QUEUE_MAX`（busy） | 指数退避后重试。 |
| `504` | 处理超 `PROCESS_TIMEOUT_SEC` | `{"error":"processing exceeded 600s, split the input"}`。 |
| `500` | pipeline 内部异常 | 服务端日志有 traceback。 |

## `GET /health`

`{"model_loaded": true, "stages_available": ["separate","denoise","vad","loudness"], "gpu": <bool>}`。

## 行为细节

- **并发与超时**：单 worker 串行，pipeline 跑在可终止子进程；**超时或调用方取消**都会按进程组
  kill 子进程（含 ffmpeg 孙进程）并真实回收后才释放锁——不会出现旧任务仍在跑、新请求又叠加。
- **模型生命周期**：模型在子进程内加载、随子进程退出释放；空闲时 GPU 零占用。Demucs device
  由 env `CLEAN_DEMUCS_DEVICE` 控制：**镜像/compose 默认 `cuda`**（GB10 实测 gpu_peak 0.91GB）；
  裸跑 `pipeline.py` 不带该 env 时回退 `cpu`。

## 示例

```bash
# 给人听：降噪 + 压低停顿（默认 48k）
curl -sS -F audio=@noisy.wav -F denoise=1 -F pause=duck \
  http://127.0.0.1:8097/clean -o out.wav -D headers.txt

# 带 BGM 视频 → 去乐人声给 ASR（separate=1，关删停顿，16k）
curl -sS -F audio=@bgm.mp4 -F separate=1 -F pause=off -F sr=16000 \
  http://127.0.0.1:8097/clean -o vocals.wav
```

Rust 消费方用 toolkit 仓的 `audio-clean-client` crate（类型化 `CleanOpts`/`CleanedAudio`、
错误归类、`/clean` 自动拼接），不要自行拼 multipart。

## 部署 / 与现有服务的关系

- 独立 `server/audio-cleanup/compose.cleanup.yaml`，端口仅 `127.0.0.1:8097`，
  `restart: unless-stopped`。**不**并入生产 `server/compose.yaml`。
- 典型串联：`/clean`（separate=1 去 BGM）→ 干净 wav →（再喂）FunASR `/transcribe` 或丢进
  `~/tts-voices/` 做克隆参考音。
