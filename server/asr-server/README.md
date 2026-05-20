# asr-server

最小可用的 ASR HTTP 服务，OpenAI Audio API 兼容形态，复用本项目同款 sherpa-onnx 模型。

## 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/healthz` | 存活检查，返回 `ok` |
| GET | `/v1/models` | 列出当前启动加载的模型 |
| POST | `/v1/audio/transcriptions` | 上传音频文件，返回识别文本 |

`POST /v1/audio/transcriptions` 接收 `multipart/form-data`：

- `file`（必填）：**16 kHz 单声道 WAV**（PCM16 / PCM32 / Float 均可）
- 其它 OpenAI 字段（`model`、`language`、`response_format`、`prompt`）当前被接收但忽略，模型/语言由启动参数固定

返回：`{ "text": "..." }`，错误：`{ "error": { "message": "...", "type": "invalid_request|server_error" } }`

## 运行

```bash
# SenseVoice 多语种（默认）
cargo run --release -- \
  --model-dir /path/to/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17 \
  --model sense-voice \
  --bind 0.0.0.0:8080

# Whisper large-v3-turbo
cargo run --release -- \
  --model-dir /path/to/sherpa-onnx-whisper-turbo \
  --model whisper-turbo \
  --language zh \
  --num-threads 4
```

`--model-dir` 下需要的文件取决于模型：

- `sense-voice`：`model.int8.onnx`、`tokens.txt`
- `whisper-turbo`：`turbo-encoder.onnx`、`turbo-decoder.onnx`、`turbo-tokens.txt`

## 调用样例

```bash
curl -F "file=@sample.wav" http://localhost:8080/v1/audio/transcriptions
```

```python
import requests
r = requests.post(
    "http://localhost:8080/v1/audio/transcriptions",
    files={"file": open("sample.wav", "rb")},
)
print(r.json()["text"])
```

OpenAI SDK 也能直接打过来（把 `base_url` 指到本服务即可）。

## 已知限制（skeleton 范围）

- **只接受 16 kHz 单声道 WAV**。MP3 / Opus / 任意采样率重采样未实现，需要在客户端先转码（`ffmpeg -ar 16000 -ac 1 in.mp3 out.wav`）或自行加 `symphonia` + `rubato`。
- **Whisper 单段 ≤ 30s**。更长音频请客户端切分或服务端加 VAD 切段（可移植 `src-tauri/src/commands/recording.rs` 的 VAD 循环）。
- **模型/语言不能按请求切换**。要支持需在启动时加载多份 `OfflineRecognizer` 并按 `model` 字段路由。
- **无鉴权**。生产环境前请加一层反向代理（nginx / Caddy）或 axum middleware 校验 `Authorization` header。
- **串行解码**。`Mutex<OfflineRecognizer>` 保证线程安全但不并发；高 QPS 需要做 recognizer pool。

## 运行时依赖

`sherpa-onnx` 用的是 `features = ["shared"]`，需要系统能找到对应的 native shared library（与 `src-tauri` 用的一致）。已在 `src-tauri` 里跑起来的机器上一般直接可用。
