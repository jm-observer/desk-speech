# pronunciation-assess — 英语发音评测(GOP)服务

GB10 `:8098 POST /assess`:参考文本 + 用户录音 → **音素 / 词 / 句三级发音分** + 定位错读音素。

- **端点契约(权威源)**:[`docs/pronunciation-assess-api.md`](../../docs/pronunciation-assess-api.md)
- **概念设计**:[`docs/pronunciation-coach-overview.md`](../../docs/pronunciation-coach-overview.md)
- **消费方**:toolkit-server 的 shadow GOP 后端(`GOP_BASE_URL=http://127.0.0.1:8098`),
  设计见 toolkit `docs/english-shadow-gop-design.md`(§3 传输分层 / §4 契约)。

> 为什么不是 ASR:通用 ASR/Whisper 抗口音、会把 "I sink" 脑补成 "I think",正好抹掉要检测的
> 错误。发音评测用**音素后验 + 强制对齐 + GOP**,是另一类语音学技术。LLM 不参与评测。

## 链路

```
ref_text ──G2P(g2p_en/CMUdict, ARPAbet)──▶ 期望音素序列
user_audio ──解码/重采样 16k──▶ wav2vec2 CTC 音素后验 ──forced_align──▶ 每音素时间段
                                                  │
                          逐音素 GOP = mean(logP_canonical − logP_argmax) ≤ 0
                                                  │
                            标定(speechocean762)──▶ 0~1 ──聚合──▶ 词 / 句
```

实现:[`gop.py`](gop.py)(引擎,torch 惰性导入)+ [`app.py`](app.py)(aiohttp 服务)。

## 模块结构

| 文件 | 职责 |
|---|---|
| `gop.py` | GOP 引擎。**上半部纯函数**(分词/标定/分档/聚合/hint/响应组装,无 torch,`test_gop.py` 直测);**下半部重推理**(模型加载缓存 + G2P + forced_align + 逐音素 GOP,torch/transformers/g2p_en 惰性导入)。 |
| `app.py` | `POST /assess`(multipart in / JSON out)+ `GET /health`。模型常驻 + `Semaphore(1)` 串行 GPU + 等待计数 `QUEUE_MAX` → 503 + 超时 → 504。 |
| `test_gop.py` / `test_app.py` | 无 torch 单测(纯逻辑 / monkeypatch 子调用)。 |
| `Dockerfile` / `compose.assess.yaml` | 薄层镜像(`FROM funasr-asr:arm64`)+ 独立 compose(绑 `127.0.0.1:8098`)。 |

## 本地开发 / 测试(无需 GPU)

```bash
# 纯逻辑 + 服务行为单测(只需 aiohttp,不拉 torch):
python server/pronunciation-assess/test_gop.py
python server/pronunciation-assess/test_app.py
```

## GB10 部署

```bash
# 在 GB10 ~/server/pronunciation-assess(经 scripts 同步,非 git checkout):
docker compose -f compose.assess.yaml up -d --build
curl -s http://127.0.0.1:8098/health        # {"model_loaded":..., "model_id":..., "gpu":true}

# 冒烟(真录音):
curl -s -F audio=@clip.webm -F ref_text="I think so" -F granularity=word \
     http://127.0.0.1:8098/assess | python3 -m json.tool
```

### ⚠️ GB10 离线部署(已实测拍板,2026-06-25)

GB10 的 **bridge 网络容器连不上外网**(透明代理 fake-ip 只对宿主 netns 生效),`--network host`
也不行。故**模型/语料在宿主预下,容器离线加载**——`compose.override.yaml`(GB10 本机,不进 git)
叠加挂载:

```yaml
services:
  pronunciation-assess:
    environment:
      - GOP_MODEL_ID=/model            # 改为本地路径
      - TRANSFORMERS_OFFLINE=1
      - HF_HUB_OFFLINE=1
      - NLTK_DATA=/extra-nltk:/usr/share/nltk_data
    volumes:
      - /home/fengqi/pa-model:/model:ro          # 宿主 curl 预下的模型
      - /home/fengqi/pa-nltk:/extra-nltk:ro       # nltk 新名 tagger
      - /home/fengqi/server/pronunciation-assess/gop.py:/app/gop.py:ro   # 快迭代,免重 build
      - /home/fengqi/server/pronunciation-assess/app.py:/app/app.py:ro
```

宿主预下(curl 走透明代理,宿主可达 hf-mirror）:

```bash
# 模型(只取 safetensors,~1.2GB)
D=~/pa-model; mkdir -p $D; B=https://hf-mirror.com/vitouphy/wav2vec2-xls-r-300m-timit-phoneme/resolve/main
for f in config.json added_tokens.json preprocessor_config.json special_tokens_map.json \
         tokenizer_config.json vocab.json model.safetensors; do curl -sL -o $D/$f $B/$f; done
# nltk 新名 tagger(nltk>=3.9 把 averaged_perceptron_tagger 改名加 _eng 后缀)
mkdir -p ~/pa-nltk/taggers && curl -sL -o /tmp/t.zip \
  https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/taggers/averaged_perceptron_tagger_eng.zip \
  && unzip -oq /tmp/t.zip -d ~/pa-nltk/taggers/

docker compose -f compose.assess.yaml -f compose.override.yaml up -d
```

### 实测(2026-06-25,GB10)

`/assess`「I think so」(CosyVoice2 中文嗓读英文,故意带口音)→ **正确区分**:AY/NG/K/S/OW 均 ~0.98 ok;
`think` 的 **TH→F**(0.0 bad)、**IH→IY**(0.15 bad)被精确定位,句分 0.62,单次 ~1.5s。
substitution 检测语言学合理(中式英语经典 th→f),证明 forced_align + GOP + IPA↔ARPAbet 桥全链路正确。
**绝对分数偏严是默认标定占位所致,待 speechocean762 真标。**

## 模型(`GOP_MODEL_ID`)

评测靠一个 **wav2vec2 CTC 音素模型**,要求输出 **ARPAbet(或可去重音规范化为 ARPAbet)的音素
token**——`gop.py` 用 `strip_stress()` 把 tokenizer vocab 与 G2P 输出对齐(大写、去重音数字)。

- 默认 **`slplab/wav2vec2-large-robust-L2-english-phoneme-recognition`**(wav2vec2-large-robust,
  **专训非母语英语**音素识别,vocab 是**小写 ARPAbet** + 每音素带 `*_err` 误读 token + 弱读 `ax`)。
  **实测远胜通用 `vitouphy/...timit-phoneme`**:连读长词(如 *delicious*)中段不再被强制对齐整段误杀
  (L 从 0.04→0.86),而 `think→sink` 这类真错读的 /θ/ 仍判 bad。换模型只改 env。
- **vocab 桥**:`gop.py` 的 `model_token_candidates()` 多候选同时兼容 ARPAbet(大小写)/ IPA;
  对**弱读 schwa** 额外认 `AH→ax`(L2 模型把弱读 AH 输出成独立 ax),`_resolve_token_ids` 取
  「同音素多写法」后验最大值。`*_err` token 暂未直接用(canonical GOP 已够;可作后续增强)。
- **上线前确认 vocab 形态**:`curl -sL https://hf-mirror.com/<id>/resolve/main/vocab.json`。
- **音频解码**:`gop.py` 用 **ffmpeg 子进程**解码任意格式(webm/opus/wav/mp4)→16k 单声道,
  **不走 `torchaudio.load`**(torchaudio 2.11 改依赖 torchcodec,且对 webm 不稳)。ffmpeg 在 base 镜像自带。

## 标定(speechocean762)

GOP 原始分是 log 后验差(`≤0`,`0`=完美),**不可直接示人**,必须标定到 `0~1`:

- `gop.py` 的 `Calibration` 用 `score01 = sigmoid(a*(gop_raw − b))`。仓库 `calibration.json` 是按
  **slplab L2 模型**真机 raw 后验手调的参考值(`a=1.2, b=−2.0, ok_min=0.6, warn_min=0.35`;L2 正确
  音素 raw≈0、弱/错音 raw≈−2~−9)。**仍是手调临时值,正式版用 speechocean762 拟合。** 换模型必重标。
- **正式标定**:在 [speechocean762](https://www.openslr.org/101/)(开源 L2 英语发音评测集,带
  音素/词/句三级人工分)上跑本引擎得到各音素 GOP 原始分,拟合 `a/b` 让模型分与人工分单调对齐,
  并据"通过线"反推 `ok_min/warn_min`。结果写 `calibration.json`,经 `GOP_CALIBRATION` env 挂载
  (compose 默认 `/calib/calibration.json`)。改标定**无需重编译/重启镜像**,重启容器即生效。
- `calibration.json` 形如:`{"a": 5.2, "b": -0.8, "ok_min": 0.68, "warn_min": 0.42}`。

## 并发 / 限额(env 可覆盖)

| env | 默认 | 含义 |
|---|---|---|
| `GOP_PORT` | 8098 | 监听端口 |
| `GOP_DEVICE` | cuda | 推理设备(cuda 不可用自动退 cpu) |
| `GOP_MODEL_ID` | timit-phoneme | 评测模型 |
| `GOP_CALIBRATION` | /calib/calibration.json | 标定参数文件(缺失用内置默认) |
| `GOP_PROCESS_TIMEOUT_SEC` | 60 | 单次评测超时 → 504(目标实际 <2s) |
| `GOP_QUEUE_MAX` | 8 | 等待队列上限,超出 → 503 |
| `GOP_CLIENT_MAX_SIZE` | 64MiB | 上传体上限 |

## 与 toolkit 的对接

toolkit-server 配 `GOP_BASE_URL=http://127.0.0.1:8098` 即切到 GOP 后端
([crates/toolkit-server/src/shadow/gop.rs](D:/git/toolkit/crates/toolkit-server/src/shadow/gop.rs))。
**未配 → toolkit 自动回退 v1-ASR 文本对齐内核**(不破现网);配了但本服务不可达 → toolkit 回 502。
契约字段两边一一对应(`sentence_score` / `words[].pron_status` / `phones[].expected_ph`…)。
