# asr-server

最小可用的 ASR HTTP 服务，OpenAI Audio API 兼容形态，复用本项目同款 sherpa-onnx 模型。

## 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/healthz` | 存活检查，返回 `ok` |
| GET | `/v1/models` | 列出当前启动加载的模型 |
| POST | `/v1/audio/transcriptions` | 上传音频文件（multipart），返回识别文本 |
| POST | `/v1/audio/transcriptions/from-source` | 传文件路径 / HTTP URL（JSON），返回识别文本 |

`POST /v1/audio/transcriptions` 接收 `multipart/form-data`：

- `file`（必填）：**任意 ffmpeg 可解的音视频文件**（mp4 / mp3 / m4a / webm / ogg /
  flac / wav…）。16 kHz 单声道 WAV 走快路径直接 hound 解码；其它格式 / 采样率 /
  声道由服务端内部 ffmpeg 转码到 16 kHz 单声道，全程不落盘。
- `vad`（可选，`true`/`false`/`1`/`0`，缺省 `false`）：开启后用 silero_vad 切段，
  逐段识别并返回 `segments[]`（见下）。
- 其它 OpenAI 字段（`model`、`language`、`response_format`、`prompt`）当前被接收但忽略，模型/语言由启动参数固定

返回：

- 不传 `vad` / `vad=false`：`{ "text": "..." }`（与旧版完全一致，向后兼容）
- `vad=true`：额外带 `segments[]`，`text` 始终存在（等价各段文本拼接）：

```json
{
  "text": "全部段拼接的文本",
  "segments": [
    {"start": 0.0, "end": 4.2, "text": "..."},
    {"start": 4.8, "end": 9.5, "text": "..."}
  ]
}
```

错误：`{ "error": { "message": "...", "type": "invalid_request|forbidden_source|not_found|endpoint_disabled|server_error" } }`

## 运行

```bash
# SenseVoice 多语种（默认）
cargo run --release -- \
  --model-dir /path/to/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17 \
  --model sense-voice \
  --bind 0.0.0.0:8091

# Whisper large-v3-turbo + 启用 from-source
cargo run --release -- \
  --model-dir /path/to/sherpa-onnx-whisper-turbo \
  --model whisper-turbo \
  --language zh \
  --num-threads 4 \
  --source-allowlist /home/fengqi/.config/zero/downloads
```

`--model-dir` 下需要的文件取决于模型：

- `sense-voice`：`model.int8.onnx`、`tokens.txt`
- `whisper-turbo`：`turbo-encoder.onnx`、`turbo-decoder.onnx`、`turbo-tokens.txt`

其它相关启动参数：

| 参数 | 默认 | 说明 |
|---|---|---|
| `--bind` | `127.0.0.1:8091` | 监听地址。**容器内须显式 `0.0.0.0:8091`**（docker 端口转发连的是容器 eth0 而非 loopback），对外暴露范围由 compose `ports` 控制 |
| `--vad-model` | `/opt/asr-server/silero_vad.onnx` | silero_vad 模型路径（镜像内由 Dockerfile COPY 进去）；缺失时 `vad=true` 请求报错 |
| `--decode-timeout` | `60` | ffmpeg 解码超时（秒），超时 kill + `decode timeout` |
| `--source-allowlist` | （空） | `from-source` 端点白名单前缀（逗号分隔）。**为空则该端点禁用** |
| `--max-source-bytes` | `104857600` | `from-source` HTTP 下载体积上限（100 MB） |
| `--source-fetch-timeout` | `30` | `from-source` HTTP 下载整体超时（秒） |

## from-source 端点

`POST /v1/audio/transcriptions/from-source` 接收 JSON，免 multipart 上传整段字节流。
**仅当启动配了 `--source-allowlist` 时启用**，否则返回 `503 endpoint_disabled`。

请求体：

```ts
{ source: string,   // "file:///abs/posix/path" | "http(s)://..."
  vad?: boolean }   // 同 multipart 的 vad
```

响应与 `/v1/audio/transcriptions` 完全一致。

- **`file://`**：只接受 `file:///<绝对 posix 路径>`（三斜杠）；含 `%` 编码字符直接 400；
  Windows 风格 `file:///C:/...` 400。路径先 `canonicalize` 再与（同样 canonical 化的）
  白名单前缀比对，防 symlink / `..` 逃逸——不匹配 `403 forbidden_source`，不存在 `404`。
  **读的是容器内路径**：跨容器调用须把宿主目录挂进容器同路径（见 compose）。
- **`http(s)://`**：流式下载到 `/tmp/asr-input/<uuid>.bin`，超 `--max-source-bytes` 或
  `--source-fetch-timeout` 立刻中止并删临时文件；处理完无论成败都删。

```bash
# 同机文件路径（zero 在 GB10 本机下载的抖音 mp4）
curl -X POST -H 'Content-Type: application/json' \
  -d '{"source":"file:///home/fengqi/.config/zero/downloads/douyin/123.mp4","vad":true}' \
  http://localhost:8091/v1/audio/transcriptions/from-source
```

## 调用样例

```bash
# 任意格式 + 不切段
curl -F "file=@sample.mp4" http://localhost:8091/v1/audio/transcriptions

# 长视频 + VAD 切段（拿 segments[] 做字幕时间轴）
curl -F "file=@clip.mp4" -F "vad=true" http://localhost:8091/v1/audio/transcriptions
```

```python
import requests
# from-source（同机路径直传，推荐给 GB10 本机调用方）
r = requests.post(
    "http://localhost:8091/v1/audio/transcriptions/from-source",
    json={"source": "file:///home/fengqi/.config/zero/downloads/douyin/123.mp4", "vad": True},
)
data = r.json()
print(data["text"])
for seg in data.get("segments", []):
    print(seg["start"], seg["end"], seg["text"])
```

OpenAI SDK 也能直接打过来（把 `base_url` 指到本服务即可，走 multipart 端点）。

## GB10 部署验证回执（2026-05-31，zero 对接）

Plan A/B/C 已部署到 GB10 并端到端实测通过（真实 49MB 抖音 mp4，sense-voice，
ffmpeg 解码 → VAD 切段 → 识别，端到端 ~22s，返回 110 个 segment）。给 zero 一侧的
对接契约确认如下：

| # | 项 | 实配 / 结论 |
|---|---|---|
| 1 | 端点 URL | `http://127.0.0.1:8091/v1/audio/transcriptions/from-source`（GB10 本机） |
| 2 | `vad=true` 出 `segments` | ✅ 见下方实测响应；每段 `{start,end,text}`，`text` 为各段拼接 |
| 3 | allowlist 实配（容器内路径） | `/home/fengqi/.config/zero/downloads`（prefix 比对，自动覆盖 `douyin/` 子目录） |
| 4 | downloads 挂载（容器内路径） | `/home/fengqi/.config/zero/downloads`（宿主同路径，只读；compose 变量 `ZERO_DOWNLOADS_DIR` 可覆盖宿主侧） |
| 5 | zero → asr-server 连法 | zero 跑 GB10 宿主进程 → `127.0.0.1:8091`（已实测）；zero 跑容器 → 加入 `server_default` 网络走 `asr-server:8091`（`127.0.0.1` 跨容器不通） |
| 6 | 除 `source`/`vad` 外需别的字段 | 无。model 由启动参数固定，不读请求体 |
| 7 | 实际 `asr_model` | `sense-voice`（SenseVoice int8） |

实测响应（节选）：

```json
{
  "text": "这盘子好温柔呀。 先看整体颜值，这个图案真精致。 这个细节越看越美。 …",
  "segments": [
    {"start": 0.088, "end": 1.46,  "text": "这盘子好温柔呀。"},
    {"start": 2.072, "end": 5.268, "text": "先看整体颜值，这个图案真精致。"},
    {"start": 5.688, "end": 7.156, "text": "这个细节越看越美。"}
  ]
}
```

> **网络拓扑**：asr-server 容器内 bind `0.0.0.0:8091`（docker 端口转发必须），host 侧
> 发布限制为 `127.0.0.1:8091`（仅 GB10 本机可达）。容器在 `server_default` 桥网络，
> IP `172.25.0.2`。**zero 部署形态（宿主进程 / 容器）决定第 5 条用哪个地址，对接前需确认。**
>
> **字幕务必带 `vad:true`**：不传或 `vad=false` 只回 `{"text":...}`（向后兼容），拿不到时间轴。

## 已知限制

- **模型/语言不能按请求切换**。要支持需在启动时加载多份 `OfflineRecognizer` 并按 `model` 字段路由。
- **无鉴权**。生产环境前请加一层反向代理（nginx / Caddy）或 axum middleware 校验 `Authorization` header。
- **串行解码**。`Mutex<OfflineRecognizer>` 保证线程安全但不并发；`vad=true` 切多段会累积
  延迟（N 段 ≈ N 倍 sherpa 延迟）。高 QPS / 低延迟需要做 recognizer pool（另立项）。
- **CPU-only**。arm64+CUDA13 的 GPU build 是后续工作（见 Dockerfile 注释）。

## 运行时依赖

`sherpa-onnx` 用的是 `features = ["shared"]`，需要系统能找到对应的 native shared library（与 `src-tauri` 用的一致）。已在 `src-tauri` 里跑起来的机器上一般直接可用。
