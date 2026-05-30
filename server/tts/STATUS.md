# TTS 服务 — 状态与待办

> 调用方接口手册看 [API.md](API.md);部署/运维看 [README.md](README.md)。
> 本文档跟踪整体项目状态、决策、待办与已知风险。

## 一句话现状(2026-05-30)

**CosyVoice2 + 预设音色库 + HTTP API 已上线,bake-off 收尾**。GPT-SoVITS
已从 compose 移除,微调流水线归档到 `legacy/`。**不接入 orchestrator/客户端**
(产品方向:TTS 作为独立服务存在,任何项目通过 `:8095` 调,见 [API.md](API.md))。

## 已开发(Done)

1. **CosyVoice2 服务**(:8095)生产化
   - 6 个端点:`GET /health` `GET /voices` `POST /tts` `POST /tts/zero_shot` `POST /tts/instruct` `POST /tts/cross_lingual`
   - `POST /tts` 是首选包装端点:voice_id-based,自动选 mode
   - `restart: unless-stopped`,重启自愈
2. **音色库** `~/tts-voices/`(容器内 `/voices`,只读挂载)
   - 6 个 ref:5 个 Edge TTS 生成(⚠️ License 限 dev)+ 1 个 CosyVoice 官方(Apache-2.0)
   - `voices.json` manifest 热可编辑(`GET /voices` 每次重读)
3. **instruct prompt enum** [instruct_prompts.json](instruct_prompts.json)
   - 实测确定有效:pace_slow/fast(强)、volume_loud/soft(中)、emotion_happy/sad/angry(需配文)
   - 实测无效已剔除:自编 OOD 字符串、`[breath]`/`[sigh]` inline tag
   - 仅 `[laughter]` inline tag 有效(`cross_lingual` 路径)
4. **bake-off 历史 A/B** 已完成
   - 个人录音方案 **放弃**:`me.wav` SNR 37dB(对比干净 ref 73dB)、峰值贴 0dBFS、节奏问题
   - GPT-SoVITS 路线 **放弃**:跨语言英文崩、短句易截断、无情感控制
5. **历史**:`server/tts/` 全量代码 + `.gitignore` + commits `1846ca4` `6fe70fd`
   + 后续 API 化改造提交

## 架构(数据流)

```
参考音频(~/tts-voices/*.wav,扁平挂载)
   └─→ CosyVoice2 :8095
         ├── /tts            (voice_id → 自动选 mode)
         ├── /tts/zero_shot  (上传 ref + 文本)
         ├── /tts/instruct   (上传 ref + 情感/语速 prompt)
         └── /tts/cross_lingual (跨语言 / [laughter])
基础镜像 funasr-asr:arm64(GB10 唯一可用的 cu130 torch),独立 compose,
与生产 asr/orchestrator 隔离。
```

GPT-SoVITS 微调流水线已归档到 `legacy/`,不再编排。

## 关键决策(为什么这么选)

- **选 CosyVoice 2**:中文零样本第一梯队、原生情感/语气指令、即时无需训练。
- **放弃 GPT-SoVITS v2Pro**:跨语言英文崩、短句易截断、无情感控制(对比详见
  bake-off 结论)。微调流水线代码归档到 `legacy/`,需要时可回溯。
- **放弃个人录音方案**:`me.wav` SNR 仅 37dB(干净 ref 73dB)、峰值贴 0dBFS、
  节奏被克隆。改走"CosyVoice2 + 预设音色库"。
- **基于 funasr-asr:arm64、不装 torch**:GB10(arm64/Blackwell sm_121)只有该基础
  镜像的 `torch 2.13.dev+cu130` 能用;装别的 torch 会废。
- **以已构建镜像为部署载体**:第三方源码不入 git;日常部署不重建、不联网。

## 待办 / 下一步可选项(TODO)

- [ ] (可选)第三个对比项:GB10 上已有 `fish-speech-webui:cuda` 镜像
      (Fish Speech / OpenAudio S1),需补权重才能跑;视需要再说。
- [ ] (可选)CosyVoice `fp16` 提速:compose 里 `COSYVOICE_FP16=1`。
- [ ] (可选)若未来转为对外/商业用途,把 `edge_*` 音色替换为 AISHELL-3
      (CC-BY-4)或 Common Voice(CC-0)。当前家庭局域网/个人用,Edge TTS
      dev/personal 授权适用。

### 已完成

- A/B bake-off(2026-05-28):选 CosyVoice2,放弃 GPT-SoVITS 与个人声纹方案。
- HTTP API 化(`/tts` + `/tts/{zero_shot,instruct,cross_lingual}` + `/voices`)。
- 音色库 `~/tts-voices/` 落地,`voices.json` 热可编辑。
- 产品方向:TTS 作为独立服务存在,**不接** orchestrator/客户端。

## 已知限制 / 注意

- 冒烟微调用的是参考音频 + 2 epoch,**音质不代表正式效果**,只证明流水线通。
- GPT-SoVITS `/tts` 即使用微调权重,**仍需传一个 ref_audio**(架构如此)。
- 国内网络抖动:GB10 构建/下载偶发失败,**重试即可**(Aliyun pip / ModelScope 可靠,
  github 不通——已用 vendor 规避)。
- 容器重建会丢热加载的微调权重(权重文件在 `~/tts-io/gs_train/<exp>/weights` 持久,
  重跑微调或手动 `/set_*_weights` 即可恢复)。

## 怎么自己上手验证

- 用 voice_id 跑一发:
  ```bash
  curl -X POST http://192.168.0.68:8095/tts \
    -H "Content-Type: application/json" \
    -d '{"text":"今天天气真不错","voice_id":"edge_yunjian"}' -o out.wav
  ```
  完整端点 + 字段见 [API.md](API.md)。
- 用自己的 ref:`POST /tts/zero_shot`(multipart),要求 5-10s、24kHz、
  SNR > 50dB、峰值 -3 ~ -6 dBFS。
- 跑英文 ref × 句子矩阵对比:`server/tts/tts-voices/syn_en_ref_clone.py`
  (输出到 `outputs/en_ref_clone/`,gitignored)。

## GB10 现场

- 容器:`tts-cosyvoice-1`(:8095)在跑,`restart: unless-stopped`,重启自愈;
  生产 `server-orchestrator-1`/`server-asr-1`/`gemma4-26b-a4b` 未受影响。
- `tts-gptsovits-1`(:8096)已从 compose 移除,不再启动。残留容器/卷可
  `docker container/volume rm` 清理(`~/gpt-sovits-assets/`、`~/gpt-sovits-cache/`
  权重/缓存可保留供回溯,占用约几 GB)。
- 部署目录:`~/server/tts/`(若要重建 sovits 镜像,需 vendor 的 `GPT-SoVITS/`
  源码,不在 git;见 `legacy/README.md`)。
- 资产:`~/funasr-prep/models/CosyVoice2-0.5B`、`~/tts-voices/`、`~/tts-io/`。
