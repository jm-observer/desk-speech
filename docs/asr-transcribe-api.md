# FunASR `/transcribe` HTTP API（离线整段转写）

> 权威源。toolkit 仓 / 其他消费方对接前以本文为准。
>
> 实现：`server/asr/app.py` 的 `http_transcribe` 处理器。与桌面端实时管线
> （WebSocket `:9100` `/stream`）共享同一份 ASR 模型，无新增 GPU 占用。

## 概览

| 项 | 值 |
|---|---|
| 方法 | `POST` |
| 路径 | `/transcribe` |
| 同机 base | `http://127.0.0.1:9101` |
| Content-Type | `multipart/form-data` |
| 模型 | 由 orchestrator 的 `asr.model` 运行时配置控制（Paraformer / SenseVoice / Whisper turbo / Whisper large-v3 任选；热切换） |
| 最大上传 | 256 MiB（aiohttp `client_max_size`，可覆盖大多数短视频音频） |
| 鉴权 | 无。仅监听 `127.0.0.1:9101`，不对 LAN 暴露——任何非本机调用方需先经过反向代理。 |
| 并发 | 串行（通过 aiohttp 的 `run_in_executor` 落到线程池；GPU 实际仍是单卡串行）。突发多请求会**排队**，不返回 429。 |

## 请求

`multipart/form-data` 字段：

| 字段 | 必填 | 类型 | 默认 | 说明 |
|---|---|---|---|---|
| `audio` | ✅ | file | — | 任意 ffmpeg 可解码的容器/编码：wav / mp3 / mp4 / m4a / webm / ogg / flac / ... 内部统一重采样到 16 kHz mono float32 后送入模型。 |
| `vad` | ❌ | str | `"1"` | `"1"`/`"true"`/`"on"`/`"yes"` → 用 FSMN-VAD 切句，返回每段时间戳。`"0"`/`"false"`/`"off"`/`"no"`/`""` → 全段一锤识别，`segments=[]`。 |

> **没有 `model` / `language` / `hotwords` 字段**：模型和热词由 orchestrator 配置统一控制
> （Web 管理台改、asr 15s 内热加载），不接受单次请求覆盖——这是有意的，避免外部
> 调用方与桌面实时管线对全局状态的并发争用。需要按管线选模型时，调 orchestrator
> 的 `/api/asr-config` 而不是给本端点加字段。

## 响应

成功 `200 OK`，`application/json`：

```json
{
  "text": "今天天气不错\n我们去公园走走",
  "segments": [
    {"t_start": 0.42, "t_end": 2.18, "text": "今天天气不错"},
    {"t_start": 2.96, "t_end": 4.51, "text": "我们去公园走走"}
  ],
  "model": "paraformer"
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `text` | string | 全文。`vad=1` 时是各 segment 文本用换行 `\n` 拼接（CJK-安全，不强加空格分词）；`vad=0` 时是单次识别原文。可能为空字符串（纯静音、全被门控掉等情况）。 |
| `segments` | array | VAD 切段结果。`vad=0` 时**恒为空数组**（不是缺省字段），调用方据此判定模式。 |
| `segments[].t_start` | f64 | 该段在原音频中的开始秒数。 |
| `segments[].t_end` | f64 | 该段结束秒数。 |
| `segments[].text` | string | 该段识别文本（已剥离 SenseVoice 的 `<\|lang\|>` 等 meta token）。 |
| `model` | string | 实际产出本次结果的模型名：`paraformer` / `sensevoice` / `whisper-turbo` / `whisper-large-v3`。便于落档 `asr_model` 字段时使用真实值而非调用方猜测。 |

## 错误

非 200 一律返回 `{"error": "<message>"}`：

| 状态 | 触发条件 | 处理建议 |
|---|---|---|
| `400` | 缺 `audio` 字段 / Content-Type 非 multipart / multipart 解析失败 | 检查上传字段名是 `audio` 不是 `file`；mp4 走二进制 part。 |
| `400` | `decode failed: ...` —— ffmpeg 解码失败（损坏/不支持的容器） | 客户端排查原文件，无需重试。 |
| `413` | 上传超过 256 MiB | 短视频极少触达；超大原始素材请先转码/截取。 |
| `500` | `recognize failed: ...` —— 模型推理异常 | 服务端日志里有完整 traceback。重试通常无效；先看 GB10 上 `docker compose logs --tail=80 asr`。 |

## 行为细节

- **`vad=1` 的分句**：与桌面实时管线共用一套 FSMN-VAD + 我们自己的 `SENTENCE_GAP_MS`
  合并规则（默认 1500 ms 静音判定为句界），所以同一段音频在桌面端实时听写得到的
  分段，与离线 `/transcribe` 得到的分段语义一致——便于离线复算结果对照线上调试。
- **首字延迟**：`vad=1` 路径要先对整段过一遍 VAD（GPU 上 1 分钟音频 < 500 ms），再
  对每个区段独立 `recognize`，总时长约为「段数 × 单段识别时间」，通常 1 分钟音频在
  GB10 上整体在 3-8 秒内（取决于活跃模型）。`vad=0` 则只跑一次 recognize，更快但
  没时间戳。
- **同模型策略**：本端点不会为了某次请求临时加载第二份模型；当前 `asr.model` 是什么就
  用什么。需要对照识别效果就调 orchestrator 配置切模型，等 ~15 s 生效后再发请求。
- **不读热词**：本端点和桌面实时管线共享 `HOTWORDS_PARAFORMER` / `HOTWORDS_WHISPER`
  全局状态——orchestrator 的 `asr.hotwords` 改了，离线 `/transcribe` 自然也吃到。

## 示例

### curl

```bash
# VAD 切段
curl -sS -F audio=@clip.mp4 -F vad=1 http://127.0.0.1:9101/transcribe | jq

# 全段一锤
curl -sS -F audio=@clip.mp3 -F vad=0 http://127.0.0.1:9101/transcribe | jq
```

### Python（同步）

```python
import requests
with open("clip.mp4", "rb") as f:
    r = requests.post(
        "http://127.0.0.1:9101/transcribe",
        files={"audio": ("clip.mp4", f, "video/mp4")},
        data={"vad": "1"},
        timeout=300,
    )
r.raise_for_status()
data = r.json()
print(data["model"], len(data["segments"]), data["text"][:80])
```

### Rust（reqwest 0.13 + tokio）

```rust
use reqwest::multipart::{Form, Part};

let bytes = tokio::fs::read("clip.mp4").await?;
let form = Form::new()
    .part("audio", Part::bytes(bytes).file_name("clip.mp4").mime_str("video/mp4")?)
    .text("vad", "1");
let resp: serde_json::Value = reqwest::Client::new()
    .post("http://127.0.0.1:9101/transcribe")
    .multipart(form)
    .send().await?
    .error_for_status()?
    .json().await?;
```

> Rust 生产消费方应使用 toolkit 仓的 `asr-client` crate，自带类型化响应、错误分类和重试策略 —— 不要自行拼 multipart。

## 部署 / 端口映射

- 路由注册在 `server/asr/app.py` 的 `main()`，与 `/embed` 同一个 aiohttp `Application` 实例，
  共用 `client_max_size=256 MiB` 上限。
- `server/compose.yaml` 把 asr 容器的 `9101` 通过 `127.0.0.1:9101:9101` 发布到宿主机
  loopback——**不**绑 `0.0.0.0`，避免无意暴露给 LAN。
- 同机外部消费（如 toolkit 抖音管线）直连 `http://127.0.0.1:9101/transcribe`；
  跨机调用需要先在反向代理上加鉴权后再开放。

## 与桌面实时管线的关系

| 方面 | 离线 `/transcribe` (本文) | 实时 `/stream` (WS `:9100`) |
|---|---|---|
| 用途 | 一次性整段转写（抖音视频、会议录音回放等） | 桌面 mic 流式听写、按句 emit |
| 协议 | HTTP multipart | WebSocket（PCM 二进制 + JSON 控制） |
| 时间戳 | 段级（`vad=1` 时） | 段级 + 流式 `segment` 事件 |
| 模型选择 | 当前 `asr.model` | 当前 `asr.model`（+ 可选 `secondary_model` 对比） |
| 鉴权 | 仅 `127.0.0.1` | 仅 LAN 内桌面端 |
| 资源 | 复用同一份已加载模型 | 同上 |

两条路径**共享同一个 ASR 进程和模型常驻**——`/transcribe` 是"既然 GPU 已经占着，
顺便对外开个离线接口"的产物，**不增加任何显存开销**。
