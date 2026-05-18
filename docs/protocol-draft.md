# 音频流协议草案(P0)

> 客户端 ↔ 编排层之间唯一契约。P0 只覆盖"音频进 / 文本出";为后续(发音评测/TTS/对话)预留扩展。
> 规格级,非最终线格式;评审通过后据此实现。

---

## 1. 设计原则

- **一条 WebSocket 连接 = 一次录音会话**(简单、状态清晰、ESP32 易实现)
- **文本帧 = 控制/事件(JSON,有 `type` 字段);二进制帧 = 音频**
- **加法式演进**:新增事件类型/字段不破坏旧客户端;`hello` 带协议版本
- VAD 在服务端(决策 E),客户端只推流

---

## 2. 会话生命周期

```
client                          server
  │ ── WS connect ──────────────▶│
  │ ── hello (JSON) ────────────▶│   声明会话参数
  │ ◀──────────── ready (JSON) ──│   服务端就绪
  │ ── audio frame (bin) ───────▶│   持续推 16k PCM
  │ ── audio frame (bin) ───────▶│
  │ ◀──────── segment (JSON) ────│   一段最终识别
  │ ◀──────── optimized (JSON) ──│   该段 LLM 优化结果
  │ ◀──────── translated (JSON) ─│   (可选)翻译
  │ ── stop (JSON) ─────────────▶│   音频结束
  │ ◀──────────── done (JSON) ───│   收尾(flush 完最后段)
  │ ── WS close ────────────────▶│
```

---

## 3. 客户端 → 服务端

### 3.1 `hello`(连接后第一帧,JSON)
| 字段 | 说明 |
|---|---|
| `type` | `"hello"` |
| `protocol` | 协议版本,如 `"1"` |
| `sample_rate` | 固定 `16000` |
| `format` | 音频编码:P0 `"pcm_s16le"`(或 `"f32le"`,二选一定死) |
| `language` | `"zh"` / `"auto"` / `"en"`…(透传给 ASR 路由,见架构 §6) |
| `want_optimize` | bool,是否要 LLM 优化文本 |
| `want_translate` | bool,是否要翻译 |

### 3.2 音频帧(二进制)
- 纯音频负载,单声道,采样率/编码同 `hello` 声明
- 建议每帧 ~20–100ms;P0 不做 Opus,先 PCM 简单优先
- 不在音频帧里塞元数据(保持二进制纯净,利于 ESP32)

### 3.3 `stop`(JSON)
- `{"type":"stop"}`:音频结束,请服务端 flush 最后未决段

---

## 4. 服务端 → 客户端(均 JSON 文本帧)

| type | 关键字段 | 含义 |
|---|---|---|
| `ready` | `session_id` | 已就绪,可开始推音频 |
| `segment` | `id`, `text`, `t_start`, `t_end` | 一段最终识别文本(VAD 切段) |
| `optimized` | `ref`(=segment.id), `text` | 该段 LLM 优化后文本 |
| `translated` | `ref`, `text` | 该段翻译(若 `want_translate`) |
| `status` | `state`(`busy`/`idle`…) | 可选,状态提示 |
| `error` | `code`, `message`, `fatal` | 错误;`fatal=true` 表示会话不可继续 |
| `done` | `session_id` | 收尾完成,客户端可关连接 |

- **关联**:`optimized`/`translated` 用 `ref` 指回 `segment.id`,客户端据此就地更新该段(对应现有 app 的 revision 思路)
- 客户端收到 `optimized` 后按本地设置**自动写剪贴板**(本地动作,不在协议内)

---

## 5. 错误 / 健壮性(P0 最小)

- 任一非致命错误 → `error`(fatal=false),会话继续
- 致命错误(模型不可用等)→ `error`(fatal=true)后服务端关连接
- **断线**:P0 视为会话结束(不做断点续传);客户端可重新建会话
- **背压**:P0 客户端按真实采集节奏推流(天然限速),不需复杂流控;后续按需加

---

## 6. 版本与扩展(为未来留口)

- `hello.protocol` 标识版本;服务端不认的字段忽略而非报错
- **预留(不在 P0 实现,仅占位约定)**:
  - `partial`(实时中间结果)
  - `assessment`(发音评测报告事件)
  - `tts_audio`(下行合成语音,二进制 + 描述帧)
  - 全双工/打断相关控制帧
- 原则:这些以后**新增 type**,P0 的 `segment/optimized/translated` 语义不变

---

## 7. P0 不做(明确)

- 不做鉴权(局域网,P0 略;后续按需加 token)
- 不做断点续传 / 多路复用(一连接一会话)
- 不做 Opus / 自适应码率(先 PCM)
- 不做 partial 实时回显(只最终段;体验不够再加 partial)
