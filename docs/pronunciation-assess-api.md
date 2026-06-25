# pronunciation-assess `/assess` HTTP API（英语发音评测 GOP）

> 权威源。toolkit 仓 shadow GOP 后端 / 其他消费方对接前以本文为准。
>
> 实现：`server/pronunciation-assess/app.py`（aiohttp）+ `gop.py`（GOP 引擎）。
> 概念设计：`docs/pronunciation-coach-overview.md`。
> 消费侧设计：toolkit `docs/english-shadow-gop-design.md`（§3 传输分层 / §4 契约）。

## 概览

| 项 | 值 |
|---|---|
| 方法 | `POST` |
| 路径 | `/assess` |
| 同机 base | `http://127.0.0.1:8098` |
| Content-Type（请求） | `multipart/form-data` |
| Content-Type（响应） | `application/json` |
| 最大上传 | 64 MiB（`GOP_CLIENT_MAX_SIZE`） |
| 鉴权 | 无。仅监听 `127.0.0.1:8098`，不对 LAN 暴露。桌面端经 toolkit-server `:8788` 代理。 |
| 并发 | 单 worker 串行（`Semaphore(1)` 串行 GPU 推理，模型常驻）；等待队列 `GOP_QUEUE_MAX=8`，满则 503。 |
| 超时 | `GOP_PROCESS_TIMEOUT_SEC=60`（单句目标实际 <2s），超时 504。 |

> **传输分层（重要）**：外部桌面端 → toolkit-server 是 **raw body + query**（不变）；
> **只有 toolkit-server → 本服务 :8098** 才是 multipart。本服务只认 multipart。

## 请求

`multipart/form-data` 字段：

| 字段 | 必填 | 类型 | 默认 | 说明 |
|---|---|---|---|---|
| `audio` | ✅ | file | — | 用户跟读录音。任意 torchaudio/ffmpeg 可解码格式（webm/opus、wav、mp3、m4a…）；服务端解码并重采样到 16k 单声道。 |
| `ref_text` | ✅ | str | — | 参考文本（用户应当读出的英文）。经 G2P 展开为期望音素序列。 |
| `granularity` | ❌ | str | `word` | `word`=返回逐词 + 逐音素 `phones[]`；`sentence`=仅句分 + 词分，**省略 `phones[]`**（省带宽/算力）。**仅裁剪返回详尽度，与消费方落库单元正交。** |
| `lang` | ❌ | str | `en` | 语言。当前仅 `en`（G2P/模型为英语）。 |

## 响应（`200`）

所有分值 **`0~1` 已标定**（与 toolkit v1 `score`/`threshold` 同区间；UI 自行 `×100` 展示）。

```jsonc
{
  "transcript": "i think so",        // 可选：CTC 反推近似文本，非稳定 ASR，仅回看用，可能缺失
  "ref_text": "I think so",
  "sentence_score": 0.71,            // 句级发音分 0~1（词分聚合）
  "words": [
    {
      "ref": "I", "score": 0.95, "pron_status": "ok",
      "phones": [ { "ph": "AY", "score": 0.95, "pron_status": "ok" } ]
    },
    {
      "ref": "think", "score": 0.42, "pron_status": "bad",
      "phones": [
        { "ph": "TH", "score": 0.18, "pron_status": "bad",
          "expected_ph": "TH", "actual_ph": "S", "hint": "/θ/ 读成了 /s/" },
        { "ph": "IH", "score": 0.71, "pron_status": "ok" }
      ]
    },
    { "ref": "so", "score": 0.88, "pron_status": "ok", "phones": [ /* … */ ] }
  ],
  "bad_phone_count": 1,              // 严重错读音素总数（始终给，sentence 粒度也据音素算）
  "model": "wav2vec2-xls-r-300m-timit-phoneme"   // 评测模型标识，落库追溯
}
```

### 字段语义

| 字段 | 类型 | 说明 |
|---|---|---|
| `transcript` | str? | **可选、非稳定**。CTC 最优解码反推的近似文本，**消费方不得依赖它做判分/对齐**。缺失时不出现该键。 |
| `ref_text` | str | 回显参考文本。 |
| `sentence_score` | float | 句级发音分 `0~1`（已标定）。 |
| `words[].ref` | str | 参考词原文（保留大小写/形态）。 |
| `words[].score` | float | 词级发音分 `0~1`。 |
| `words[].pron_status` | str | 发音三档：`ok` 达标 / `warn` 偏弱 / `bad` 明显错读。 |
| `words[].phones` | array? | 逐音素明细。`granularity=sentence` 时**省略**。 |
| `phones[].ph` | str | 期望音素（ARPAbet，如 `TH`）。 |
| `phones[].score` | float | 该音素发音分 `0~1`。 |
| `phones[].pron_status` | str | 发音四档:`ok`/`warn`/`bad`/**`uncertain`**(引擎没把这个音对齐好,不判对错)。 |
| `phones[].expected_ph` | str? | 错读时的「期望音素」（结构化，**消费方/落库以此为准**）。`ok` 音素省略。 |
| `phones[].actual_ph` | str? | 错读时的「实际最可能音素」。无明确替代（漏读/偏弱）时省略。 |
| `phones[].hint` | str? | 人类可读纠音文案，由 `expected_ph`/`actual_ph` 拼出（如「/θ/ 读成了 /s/」）。**仅展示用**，消费方可自行本地化，不要解析它取信息。 |
| `phones[].reliable` | bool? | `false` → 该音素**没对齐好**(`uncertain`),不计入 `bad_phone_count`、不拉低词分。`ok` 音素省略(默认可靠)。见 `english-shadow-scoring-ui-design.md` §3。 |
| `phones[].t_start` / `t_end` | float? | 该音素对齐时间段(秒),供明细表/波形定位。 |
| `phones[].peak_t` | float? | 诊断:该音素**全局峰时间**(秒)。落在 `[t_start,t_end]` 外 = 对齐错位(明细表据此标"错位")。 |
| `phones[].gop_raw` | float? | 诊断:对齐段内 canonical 峰值 log 后验(原始 GOP,≤0,越接近 0 证据越强)。 |
| `bad_phone_count` | int | `pron_status==bad` 的音素总数。供「通过判定」（消费方 `passed = sentence_score>=阈值 && bad_phone_count==0`）。 |
| `model` | str | 评测模型标识。 |

> `pron_status` **刻意不叫 `status`**：toolkit 端 v1 已有 `status`（`ok/wrong/missing`，语义是
> 内容对错），两者是独立维度，不可混用。消费方据 `pron_status` 上色，回退时用 `status`。

## 错误

| 状态码 | 触发 | body |
|---|---|---|
| `400` | 非 multipart / 缺 `audio` / 缺 `ref_text` / `granularity` 非法 | `{"error": "..."}` |
| `500` | 评测内部异常（解码失败 / 推理报错） | `{"error": "assess failed: ..."}` |
| `503` | 等待队列满（`>= GOP_QUEUE_MAX`） | `{"error": "busy"}` |
| `504` | 单次评测超过 `GOP_PROCESS_TIMEOUT_SEC` | `{"error": "assessment exceeded 60s"}` |

> **消费方约定**（toolkit-server）：`GOP_BASE_URL` **未配 → 回退 v1-ASR 内核**（不破现网）；
> 配了但本服务不可达 / 报错 → toolkit 回 **502**。

## `GET /assess/stream`(WebSocket,流式发音评测)

边收音频边出**临时**逐词分(partial),整句结束用批量 `/assess` 出**权威分**(final)。
实现:`server/pronunciation-assess/streaming.py`(StreamingAssessor)+ `app.py`。
消费侧设计:toolkit `docs/english-shadow-realtime-design.md` §6。

**上行**:
| 帧 | 内容 |
|---|---|
| `hello`(JSON,首帧) | `{ "type":"hello", "ref_text":"I think so", "granularity":"word"\|"sentence" }` |
| audio(二进制) | 16k 单声道 PCM **s16le**,建议每帧 ~200–320ms |
| `end`(JSON) | `{"type":"end"}` → 触发批量 finalize |

**下行**(JSON 事件):
| type | 关键字段 | 含义 |
|---|---|---|
| `ready` | — | 已就绪(模型加载完),可推音频 |
| `partial` | `word_index`、`ref`、`score`、`pron_status`、`phones?`、`final:false` | **逐词落定**的临时分(committed→partial);`granularity=word` 带 `phones[]` |
| `final` | `{sentence_score, words[], bad_phone_count, model}`(= 批量 `/assess` 响应) | 整句权威分,覆盖临时分 |
| `error` | `message` | 错误 |

> **临时分语义(重要)**:`partial` 只在音素「落定」(在线 Viterbi 前沿越过 commit 帧)后发,
> **落定后稳**;但 commit 前的 live 抖动(尤其错读音素)**不发**,交前端 tentative 渲染。
> 尾部未及落定的词仅在 `final` 出现。落库/通过判定**以 `final` 为准**。

**最小调用**(Python,见 `test_stream_client.py`):
```
hello → 循环 send_bytes(PCM 320ms 块)→ end;读 ready/partial*/final。
```

## `GET /health`

```jsonc
{ "model_loaded": true, "model_id": "wav2vec2-...-timit-phoneme", "gpu": true }
```

`model_loaded` 在首个 `/assess` 触发模型加载后变 `true`（模型常驻）。

## 算法 / 标定要点

- **G2P**：`g2p_en`（内置 CMUdict + OOV seq2seq 兜底），输出 ARPAbet（去重音）。
- **声学模型**：wav2vec2 CTC 音素后验（`GOP_MODEL_ID`，须输出 ARPAbet 可规范化的 token）。
- **强制对齐**：`torchaudio.functional.forced_align`（CTC），得每音素帧区间。
- **GOP**：`mean_t(logP(canonical) − logP(argmax))`（`≤0`，`0`=完美）。
- **标定**：`sigmoid(a*(gop−b))` → `0~1`，参数经 speechocean762 拟合，存 `calibration.json`
  （`GOP_CALIBRATION` 挂载，改之无需重编译）。详见 `server/pronunciation-assess/README.md`。
