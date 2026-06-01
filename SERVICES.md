# StreamSpeech 服务 API 索引

GB10(`192.168.0.68`)上对外暴露的三组服务。本文是**调用方手册的入口**:列端点、
给最小 curl 示例,详细规格指向各自的专项文档。

| 服务 | 端口 | 协议 | 谁在调 | 详细规格 |
|---|---|---|---|---|
| **orchestrator** | `8090` | WebSocket `/stream` + HTTP `/api/*` | 桌面客户端、管理台 | [§1](#1-orchestrator-8090) · [`docs/protocol-draft.md`](docs/protocol-draft.md) |
| **asr-server** | `8091` | HTTP(OpenAI Audio API 兼容) | 任何外部项目 | [§2](#2-asr-server-8091) · [`server/asr-server/README.md`](server/asr-server/README.md) |
| **CosyVoice2 TTS** | `8095` | HTTP(JSON + multipart) | 任何外部项目 | [§3](#3-cosyvoice2-tts-8095) · [`server/tts/API.md`](server/tts/API.md) |

> 内部服务(不对外):`asr` 容器的 `ws://asr:9100`(orchestrator 专用)、
> `http://asr:9101/embed`(声纹注册,经 orchestrator 代理)、vLLM `:8085`(润色/翻译,
> orchestrator 经 `host.docker.internal` 调)。

---

## 1. orchestrator(`:8090`)

桌面客户端的会话服务,**WebSocket 流式 ASR + 段后 LLM 润色/翻译**。同时挂着 Web
管理台和 HTTP 管理 API。

### 1.1 WebSocket `/stream` —— 一次录音会话

完整协议见 [`docs/protocol-draft.md`](docs/protocol-draft.md)。简版:

```
client                           server
  │ ── WS connect ──────────────▶│
  │ ── hello (JSON) ────────────▶│  声明会话参数
  │ ◀──────────── ready (JSON) ──│
  │ ── audio (binary, 16k PCM) ─▶│  持续推
  │ ◀──────── segment (JSON) ────│  一段最终识别(VAD 切段)
  │ ◀──────── optimized (JSON) ──│  该段 LLM 润色(可选)
  │ ◀──────── translated (JSON) ─│  该段翻译(可选)
  │ ◀──────── secondary (JSON) ──│  次模型对比识别(可选)
  │ ── stop (JSON) ─────────────▶│
  │ ◀──────────── done (JSON) ───│
```

**上行**

| 帧 | 内容 |
|---|---|
| `hello`(JSON,第一帧) | `{ "type":"hello", "protocol":"1", "sample_rate":16000, "format":"pcm_s16le", "language":"zh"\|"auto"\|"en", "want_optimize":bool, "want_translate":bool, "want_secondary":bool }` |
| audio(二进制) | 单声道 16kHz PCM s16le,建议每帧 20–100ms |
| `stop` / `reset`(JSON) | `{"type":"stop"}` / `{"type":"reset"}` |

**下行**(所有事件均 JSON 文本帧,`type` 区分)

| type | 关键字段 | 含义 |
|---|---|---|
| `ready` | `session_id` | 已就绪 |
| `segment` | `id`、`text`、`t_start`、`t_end`、`speaker?` | 一段最终识别 |
| `optimized` | `ref`(= segment.id)、`text` | 该段 LLM 润色 |
| `translated` | `ref`、`text` | 该段翻译 |
| `secondary` | `ref`、`text`、`kind?` | 次模型对比识别(仅识别,不进润色/翻译) |
| `error` | `code`、`message`、`fatal` | 错误;`fatal=true` 关连接 |
| `done` | `session_id` | 收尾完成 |

**最小调用示例**(浏览器 / Node):

```js
const ws = new WebSocket("ws://192.168.0.68:8090/stream");
ws.binaryType = "arraybuffer";
ws.onopen = () => ws.send(JSON.stringify({
  type: "hello", protocol: "1", sample_rate: 16000, format: "pcm_s16le",
  language: "zh", want_optimize: true, want_translate: false, want_secondary: false,
}));
ws.onmessage = (e) => {
  if (typeof e.data !== "string") return;          // 服务端不会发二进制
  const ev = JSON.parse(e.data);
  console.log(ev.type, ev);
};
// 持续 send(Int16Array.buffer) 单声道 16kHz PCM
// 结束:ws.send(JSON.stringify({type:"stop"}))
```

### 1.2 HTTP `/api/*` —— 管理台 / 段记录 / 运行时配置

| 方法 | 路径 | 用途 |
|---|---|---|
| `GET`    | `/` · `/segment/:id` | Web 管理台(HTML) |
| `GET`    | `/api/stats` | 计数(段数/说话人数等) |
| `GET`    | `/api/history?limit=200` | 最近段记录(JSON 数组) |
| `GET`    | `/api/asr-config` | 运行时 asr 配置快照(asr 容器每 ~15s 拉一次) |
| `GET`/`POST` | `/api/config` | 全量 KV 配置(`asr.*`、`vllm.*`、`llm.*` 等) |
| `GET`    | `/api/speakers` · `/api/voiceprints` | 说话人 / 启用的声纹 |
| `POST`   | `/api/speakers/enroll?name=...` (body = WAV) | 注册声纹(转 asr `/embed`) |
| `DELETE` | `/api/speakers/:id` | 删除说话人 |
| `POST`   | `/api/speakers/:id/rename` · `/enabled` | 重命名 / 启停 |
| `GET`    | `/api/segments/:id` | 单段 JSON |
| `GET`    | `/api/segments/:id/audio` | 单段音频(WAV;>1 天后失效) |
| `POST`   | `/api/segments/:id/text` | 修正段文本(构造 (audio,text) 校正样本) |
| `POST`   | `/api/segments/:id/rerun` | 用当前 prompt + vLLM 重跑润色/翻译 |
| `DELETE` | `/api/segments/:id` | 删段 |
| `DELETE` | `/api/segments` | 清空全部段(破坏性,UI 已二次确认) |

**冒烟**:

```bash
curl -s http://192.168.0.68:8090/api/stats
curl -s 'http://192.168.0.68:8090/api/history?limit=5'
```

---

## 2. asr-server(`:8091`)

**独立的 OpenAI Audio API 兼容 HTTP**,用 sherpa-onnx,跟 orchestrator 完全解耦。
OpenAI SDK 可直接打过来,把 `base_url` 指到本服务。

> ⚠️ **默认仅 GB10 本机可达**:from-source 上线后端口发布收紧为 `127.0.0.1:8091`
> (compose `ports`),局域网 `192.168.0.68:8091` **打不通**。同机调用方用 `http://127.0.0.1:8091`;
> 要对局域网开放需显式改 compose `ports` 为 `0.0.0.0` / 具体网卡。
>
> ⚠️ 此服务走 `profiles: [asr-server]`,**不随生产栈自动启动**;需
> `docker compose --profile asr-server up -d asr-server` 显式拉起。详见
> [`server/asr-server/README.md`](server/asr-server/README.md)。

### 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET`  | `/healthz` | 存活检查,返回 `ok` |
| `GET`  | `/v1/models` | 列出启动时加载的模型 |
| `POST` | `/v1/audio/transcriptions` | multipart 上传**任意音视频**,返回 `{ "text": "..." }`(可选 `vad`) |
| `POST` | `/v1/audio/transcriptions/from-source` | JSON 传**文件路径 / URL**,免上传整段字节流(同机调用首选) |

### `POST /v1/audio/transcriptions` —— multipart 上传

| 字段 | 类型 | 说明 |
|---|---|---|
| `file` | file(必填) | **任意 ffmpeg 可解的音视频**(mp4/mp3/m4a/webm/ogg/flac/wav…);16k 单声道 WAV 走快路径,其余服务端内部 ffmpeg 转码 |
| `vad` | `true`/`false`(可选,默认 `false`) | 开启后 silero_vad 切段,逐段识别,**额外返回 `segments[]`** |
| `model` / `language` / `response_format` / `prompt` | string | 接收但忽略;模型/语言由启动参数固定 |

```bash
# 任意格式 + 不切段（默认仅本机可达，故用 127.0.0.1）
curl -F "file=@clip.mp4" http://127.0.0.1:8091/v1/audio/transcriptions
# 长视频 + VAD 切段(拿 segments[] 做字幕时间轴)
curl -F "file=@clip.mp4" -F "vad=true" http://127.0.0.1:8091/v1/audio/transcriptions
```

### `POST /v1/audio/transcriptions/from-source` —— 路径 / URL 直传

**application/json**;**仅当服务端配了 `--source-allowlist` 时启用**,否则 `503 endpoint_disabled`。

```ts
{ source: string,   // "file:///abs/posix/path" | "http(s)://..."
  vad?: boolean }    // 同上
```

- `file://`:只接受 `file:///<绝对 posix 路径>`;canonical + 白名单前缀校验(防 `..`/symlink),
  含 `%` 或 Windows 风格 → 400,不在白名单 → 403,不存在 → 404。**读的是容器内路径**。
- `http(s)://`:流式下载(体积 / 超时上限),处理完删临时文件。

**响应**(`vad=true` 时带 `segments[]`,`text` 始终存在;不传 `vad` 仅 `{"text":...}`,向后兼容):

```json
{
  "text": "全部段拼接的文本",
  "segments": [
    {"start": 0.088, "end": 1.46, "text": "这盘子好温柔呀。"},
    {"start": 2.072, "end": 5.268, "text": "先看整体颜值，这个图案真精致。"}
  ]
}
```

```bash
curl -X POST http://127.0.0.1:8091/v1/audio/transcriptions/from-source \
  -H 'Content-Type: application/json' \
  -d '{"source":"file:///home/fengqi/.config/zero/downloads/douyin/<aweme_id>.mp4","vad":true}'
```

> **zero 对接契约**(2026-05-31 GB10 实测通过,真实 mp4 → 110 段):allowlist 实配
> `/home/fengqi/.config/zero/downloads`(覆盖 `douyin/` 子目录);compose 已把宿主该目录挂进
> 容器**同路径**(只读)。**网络**:容器内 bind `0.0.0.0:8091`,host 发布限 `127.0.0.1:8091`——
> zero 跑 GB10 宿主进程走 `127.0.0.1:8091`(已实测);zero 跑容器则需加入 `server_default`
> 网络走 `asr-server:8091`(127.0.0.1 跨容器不通)。**做字幕务必带 `vad:true`**。
> 完整回执 + 启动参数见 [`server/asr-server/README.md`](server/asr-server/README.md)。

### 错误 & 已知限制

- 错误体:`{ "error": { "message": "...", "type": "invalid_request|forbidden_source|not_found|endpoint_disabled|server_error" } }`
- ~~只接受 16k WAV~~ / ~~Whisper 单段 ≤30s~~ 已解决(ffmpeg 多格式 + VAD 切段)
- **模型/语言不能按请求切换**(由启动参数固定);**无鉴权**;**串行解码**(`vad=true` 多段累积延迟)
- arm64+CUDA13 上当前是 CPU-only(sherpa-onnx 上游 CUDA prebuilt 仅到 12.x)

---

## 3. CosyVoice2 TTS(`:8095`)

**独立 TTS HTTP 服务**,任何项目通过 `http://192.168.0.68:8095` 调用即可生成语音。
与 orchestrator/asr 完全隔离(独立 compose、独立镜像)。

> 完整手册见 [**`server/tts/API.md`**](server/tts/API.md)(端点细节 + voice_id 清单
> + 有效 instruct enum + Python/curl 示例 + License)。

### 端点

| 方法 | 路径 | 输入 | 用途 |
|---|---|---|---|
| `GET`  | `/health`            | — | 健康检查 |
| `GET`  | `/voices`            | — | 列出可用音色(热可编辑,每次重读 manifest) |
| `POST` | `/tts`               | JSON | **首选**:voice_id-based,自动选 mode |
| `POST` | `/tts/zero_shot`     | multipart | 上传自定义 ref wav |
| `POST` | `/tts/instruct`      | multipart | 上传 ref + 情感/语速控制 |
| `POST` | `/tts/cross_lingual` | multipart | 上传 ref + 跨语言 / `[laughter]` |

**99% 调用走 `POST /tts`**。

### `POST /tts` —— JSON 请求

```json
{
  "text":         "需要合成的文本",         // 必填
  "voice_id":     "edge_yunjian",          // 必填,见 GET /voices
  "instruct":     "请非常开心地说一句话。",  // 可选,情感/语速控制
  "prompt_text":  null,                    // 可选,override 服务端默认
  "mode":         null                     // 可选,强制 zero_shot/instruct/cross_lingual
}
```

返回:`audio/wav`(24kHz mono 16-bit)。

**Mode 自动选择**:`mode` 显式 > `instruct` 非空 → `instruct` > text 含 `[laughter]`
→ `cross_lingual` > 默认 `zero_shot`。**有效 instruct enum 见
[`server/tts/instruct_prompts.json`](server/tts/instruct_prompts.json)**(自编字符串无效)。

**示例**:

```bash
# 1. 普通
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"今天天气真不错","voice_id":"edge_yunjian"}' -o out.wav

# 2. 情感(开心)
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"太好了！我们提前完成任务了！","voice_id":"edge_yunjian","instruct":"请非常开心地说一句话。"}' \
  -o happy.wav

# 3. 笑声
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"哎呀这个真的太搞笑了[laughter]我都笑哭了","voice_id":"edge_yunjian"}' \
  -o laugh.wav
```

```python
import requests
r = requests.post(
    "http://192.168.0.68:8095/tts",
    json={"text": "你好世界", "voice_id": "edge_yunjian"},
    timeout=180,
)
r.raise_for_status()
open("out.wav", "wb").write(r.content)
```

### 自定义 ref(上传自己的声音)

走 multipart 三端点之一(详见 [`server/tts/API.md`](server/tts/API.md)):

```bash
curl -X POST http://192.168.0.68:8095/tts/zero_shot \
  -F "tts_text=今天天气真不错" \
  -F "prompt_text=希望你以后能够做的比我还好呦。" \
  -F "prompt_wav=@my_voice.wav" \
  -o out.wav
```

ref wav 建议:**5–10s、24kHz、SNR > 50dB、峰值 -3 ~ -6 dBFS**(SNR 不够会显著降质)。

### 错误码

| HTTP | 含义 |
|---|---|
| `200` | OK,返回 WAV |
| `400` | 请求格式错(如 mode 取值不在白名单) |
| `404` | `voice_id` 不在音色库 |
| `422` | 字段验证失败 |
| `500` | 模型推理异常 / 磁盘上 wav 文件缺失 |

### 注意

- **首次请求慢**:模型懒加载 ~5GB 进显存(约 30s),`/health.model_loaded` 变 `true` 后稳定 < 5s/请求
- **无并发隔离**:GPU-bound,串行调用最稳;调用方自己排队
- **音色 License**:`edge_*` 为 Microsoft Edge TTS dev/personal(局域网/个人 OK,
  对外/商业需替换为 AISHELL-3 或 Common Voice),`cosy_*` Apache-2.0
- **加新音色**:wav 丢进 GB10 `~/tts-voices/` + 改 `voices.json`,**不用重启容器**

---

## 端口速查

| 端口 | 服务 | 是否对外 | 备注 |
|---|---|---|---|
| `8090` | orchestrator(WS + 管理台 + `/api/*`) | ✅ | 桌面客户端连这个 |
| `8091` | asr-server(OpenAI 兼容 HTTP) | ⚠️ 需 profile 启动 + 默认仅 `127.0.0.1` | 同机 ASR 调用(zero 等);LAN 需改 compose `ports` |
| `8095` | CosyVoice2 TTS | ✅ | 外部 TTS 调用 |
| `8085` | vLLM(host 进程) | 内部 | orchestrator 经 `host.docker.internal` 调 |
| `9100` / `9101` | asr 容器内部 WS + `/embed` | 内部 | 仅 orchestrator 调,不暴露 |
| `8096` | ~~GPT-SoVITS~~ | ❌ 已停 | bake-off 后移除,代码归档到 `server/tts/legacy/` |

---

## 进一步阅读

- 桌面客户端 ↔ orchestrator 协议:[`docs/protocol-draft.md`](docs/protocol-draft.md)
- 系统总览 / 端口 / 部署:[`docs/redesign-architecture-overview.md`](docs/redesign-architecture-overview.md) · [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md)
- TTS 部署运维 / 音色库:[`server/tts/README.md`](server/tts/README.md)
- TTS 项目状态 / bake-off 决策:[`server/tts/STATUS.md`](server/tts/STATUS.md)
- asr-server 部署 / 模型切换:[`server/asr-server/README.md`](server/asr-server/README.md)
