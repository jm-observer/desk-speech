# CosyVoice2 TTS — HTTP API

独立 TTS 服务,任何项目通过 HTTP 调用即可生成语音。
**Base URL**: `http://192.168.0.68:8095`
**Auth**: 无(内网服务)
**Engine**: CosyVoice2-0.5B on GB10 (NVIDIA arm64 + CUDA13)

部署/运维细节请看 [README.md](README.md)。本文是给**调用方**的接口手册。

---

## 一句话上手

```bash
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"你好，这是 TTS 测试","voice_id":"edge_yunjian"}' \
  -o out.wav
```

收到 WAV 文件,默认 24kHz mono 16-bit。

---

## Endpoint 总览

| Method | Path | 输入 | 用途 |
|---|---|---|---|
| `GET`  | `/health`            | — | 健康检查 |
| `GET`  | `/voices`            | — | 列出可用音色 |
| `POST` | `/tts`               | JSON | **首选**:用 voice_id 引用音色库,自动选 mode |
| `POST` | `/tts/zero_shot`     | multipart | 上传你自己的 ref wav |
| `POST` | `/tts/instruct`      | multipart | 上传 ref + 情感/语速控制 |
| `POST` | `/tts/cross_lingual` | multipart | 上传 ref + 跨语言/inline tag |

**99% 调用走 `POST /tts`**,只有要用自己的 ref 声音才用底下三个 multipart 端点。

---

## `GET /health`

```bash
curl http://192.168.0.68:8095/health
```

```json
{
  "ok": true,
  "model_loaded": true,
  "fp16": false,
  "voices_dir": "/voices",
  "voice_count": 6
}
```

`model_loaded` 首次启动是 `false`,在第一次合成请求后变 `true`(模型懒加载)。

---

## `GET /voices`

返回音色库完整元数据。

```bash
curl http://192.168.0.68:8095/voices
```

```json
{
  "prompt_text": "今天的会议讨论了下个季度的重点工作和团队分工安排。",
  "voices": [
    {
      "id": "edge_yunjian",
      "file": "edge_yunjian.wav",
      "gender": "M",
      "tone": "中性",
      "source": "edge-tts",
      "source_voice": "zh-CN-YunjianNeural",
      "license": "Microsoft Edge TTS (dev/personal use)"
    },
    ...
  ]
}
```

**字段说明**:
- `id` — 调用 `POST /tts` 时传给 `voice_id` 字段
- `prompt_text` — 该 ref wav 的逐字转写;调用方不用关心,服务端自动用
- `prompt_text_override`(可选)— 个别 voice 有自己的 prompt_text,覆盖顶层默认值
- `license` — **生产前必查**(部分音色仅 dev/personal,见下方"⚠️ License")

**当前音色清单**(实际以 `GET /voices` 为准):

| voice_id | 性别 | 音色 | License |
|---|---|---|---|
| `edge_xiaoxiao` | F | 温暖 | Edge TTS (dev) ⚠️ |
| `edge_xiaoyi`   | F | 活泼 | Edge TTS (dev) ⚠️ |
| `edge_yunxi`    | M | 活泼 | Edge TTS (dev) ⚠️ |
| `edge_yunjian`  | M | 中性 | Edge TTS (dev) ⚠️ |
| `edge_yunyang`  | M | 严肃 | Edge TTS (dev) ⚠️ |
| `cosy_zero_shot`| F | 活泼 | Apache-2.0 ✅ |

---

## `POST /tts` — 首选端点

JSON body,通过 `voice_id` 引用服务端音色库。服务端自动:
- 查 `voices.json` 找到对应 wav 和 prompt_text
- 根据 payload 字段选 mode(`zero_shot` / `instruct` / `cross_lingual`)
- 拼出 CosyVoice2 的内部调用,返回 WAV bytes

### Request

```json
{
  "text":         "需要合成的文本",          // 必填
  "voice_id":     "edge_yunjian",            // 必填,音色 id
  "instruct":     "请非常开心地说一句话。",   // 可选,带情感/语速控制
  "prompt_text":  null,                      // 可选,override 服务端默认
  "mode":         null                       // 可选,强制 mode
}
```

### Mode 自动选择规则(优先级从上到下)

| 条件 | mode |
|---|---|
| `mode` 字段显式指定 | 用该值 |
| `instruct` 字段非空 | `instruct` |
| `text` 含 `[laughter]` | `cross_lingual` |
| 否则 | `zero_shot` |

### 三个 mode 的区别

| mode | 用途 | 必传字段 |
|---|---|---|
| `zero_shot` | 普通"用这个声音念这段话" | text + voice_id |
| `instruct` | 加情感/语速/音量 | text + voice_id + **instruct** |
| `cross_lingual` | 处理 `[laughter]` / 跨语言 | text + voice_id |

### 有效的 `instruct` 取值

只有训练集里的 prompt 真起作用(自己写的会被当文本读出来——`cosy_server.py` 会自动加 `"You are a helpful assistant. ... <|endofprompt|>"` 包装,但**模型只认得这 26 条里的内容**)。强度从实测:

| 类别 | instruct 字符串 | 强度 |
|---|---|---|
| 语速 | `请用尽可能慢地语速说一句话。` | 强 |
| 语速 | `请用尽可能快地语速说一句话。` | 强 |
| 音量 | `Please say a sentence as loudly as possible.` | 中 |
| 音量 | `Please say a sentence in a very soft voice.` | 中 |
| 情感 | `请非常开心地说一句话。` | 弱(配 lex-happy 文本时强) |
| 情感 | `请非常伤心地说一句话。` | 弱(配 lex-sad 文本时强) |
| 情感 | `请非常生气地说一句话。` | 弱(配 lex-angry 文本时强) |

完整 enum 见 [instruct_prompts.json](instruct_prompts.json)。**自己编的 instruct 字符串无效**。

### Inline tag(在 `text` 里直接写)

| Token | 实测 | 路由 |
|---|---|---|
| `[laughter]` | ✅ 有效 | 自动选 cross_lingual |
| `[breath]`   | ❌ 无声音 | — |
| `[sigh]`     | ❌ 无声音 | — |

用法:`"哎呀这个真的太搞笑了[laughter]我笑哭了"`(**前后不留空格**)。

### 错误码

| HTTP | 含义 |
|---|---|
| 200 | OK,返回 WAV |
| 400 | 请求格式错(如 mode 取值不在白名单) |
| 404 | `voice_id` 不在音色库 |
| 422 | pydantic 验证失败(漏字段或字段类型不对) |
| 500 | 模型推理异常 / 磁盘上的 wav 文件缺失 |

### 示例

```bash
# 1. 最简:用某个音色读一段话
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"今天天气真不错","voice_id":"edge_yunjian"}' \
  -o out.wav

# 2. 加情感(开心地说)
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"太好了！我们提前完成任务了！","voice_id":"edge_yunjian","instruct":"请非常开心地说一句话。"}' \
  -o happy.wav

# 3. 加笑声
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"哎呀这个真的太搞笑了[laughter]我都笑哭了","voice_id":"edge_yunjian"}' \
  -o laugh.wav

# 4. 慢速朗读
curl -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"请仔细听清楚每一个字","voice_id":"edge_yunjian","instruct":"请用尽可能慢地语速说一句话。"}' \
  -o slow.wav
```

```python
# Python (requests)
import requests

r = requests.post(
    "http://192.168.0.68:8095/tts",
    json={"text": "你好世界", "voice_id": "edge_yunjian"},
    timeout=180,
)
r.raise_for_status()
with open("out.wav", "wb") as f:
    f.write(r.content)
```

---

## `POST /tts/zero_shot` — 用自己的 ref 声音

multipart form,适合"我有一段自己的录音想克隆"的高级场景。
注意:`prompt_text` 必须是 `prompt_wav` 的**逐字转写**,搞错会显著降低音质。

```bash
curl -X POST http://192.168.0.68:8095/tts/zero_shot \
  -F "tts_text=今天天气真不错" \
  -F "prompt_text=希望你以后能够做的比我还好呦。" \
  -F "prompt_wav=@my_voice.wav" \
  -o out.wav
```

| Form 字段 | 类型 | 说明 |
|---|---|---|
| `tts_text` | string | 要合成的文本 |
| `prompt_text` | string | ref wav 的逐字转写,**必填** |
| `prompt_wav` | file | ref 音频文件(建议 5-10s、24kHz、SNR > 50dB、峰值 -3 ~ -6 dBFS) |

---

## `POST /tts/instruct` — 用自己的 ref + 情感控制

```bash
curl -X POST http://192.168.0.68:8095/tts/instruct \
  -F "tts_text=太好了完成任务了" \
  -F "instruct=请非常开心地说一句话。" \
  -F "prompt_wav=@my_voice.wav" \
  -o out.wav
```

注意:不需要 `prompt_text`(instruct 自带条件)。服务端会自动给 `instruct` 字段加
`"You are a helpful assistant. ... <|endofprompt|>"` 包装,**调用方传自然语言即可**。

---

## `POST /tts/cross_lingual` — 用自己的 ref + 跨语言/inline tag

```bash
curl -X POST http://192.168.0.68:8095/tts/cross_lingual \
  -F "tts_text=哎呀这个真的太搞笑了[laughter]" \
  -F "prompt_wav=@my_voice.wav" \
  -o out.wav
```

不需要 `prompt_text` 也不需要 `instruct`。适用场景:
- text 里含 `[laughter]`(其他 inline tag 无效)
- 中文 ref 念英文 / 反过来

---

## License

| 音色前缀 | License | 当前用法是否 OK |
|---|---|---|
| `edge_*` | Microsoft Edge TTS — dev/personal use | ✅ 个人/局域网/学习/原型 |
| `cosy_*` | Apache-2.0 | ✅ 无限制 |

**当前部署**:家庭局域网、仅个人使用 → 全部音色都在授权范围内。

**未来若转为对外/商业用途**,需要把 `edge_*` 替换成 AISHELL-3(CC-BY-4)或
Common Voice(CC-0),操作流程参考 [STATUS.md](STATUS.md) 历史决策。

---

## FAQ

**Q: 第一次请求很慢?**
A: 模型懒加载,首次请求会加载 ~5GB 进显存(约 30 秒)。`/health` 的 `model_loaded` 变 `true` 后就稳定 < 5s/请求。

**Q: 并发?**
A: 没做并发隔离。模型推理是 GPU-bound,串行调用最稳。建议调用方串行排队,或加上游限流。

**Q: 输出音频格式?**
A: 24kHz mono 16-bit WAV(`Content-Type: audio/wav`)。

**Q: 最大文本长度?**
A: CosyVoice2 支持长文本(模型内部分段),但建议单次 < 200 字以保证延迟可控。

**Q: 用自己录音作 ref 效果不好?**
A: 大概率是录音问题(背景噪音 / 峰值贴 0dBFS / 语速过慢)。
要求:5-10s、24kHz、SNR > 50dB、峰值 -3 ~ -6 dBFS、内容均匀无长停顿。
诊断方法见 [STATUS.md](STATUS.md) 的"已知限制"小节。

**Q: 想加新音色?**
A: 把 wav 放到 `~/tts-voices/` 下,在 `~/tts-voices/voices.json` 加一条 entry,
**不用重启容器** —— `/voices` 每次请求重读 manifest。
