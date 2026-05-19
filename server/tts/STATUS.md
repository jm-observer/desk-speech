# TTS bake-off — 状态与待办(给人看的总览)

> 目的:你先理解「已经做了什么 / 还差什么 / 下一步可选项」。
> 操作细节看同目录 `README.md`(部署 runbook)。

## 一句话现状

为给项目选 TTS 方案,已在 GB10 搭好**两个可用的独立服务**并跑通验证;
代码已提交分支 `redesign/client-server`。**尚未**接入主产品(orchestrator/客户端),
也**尚未**用你本人的声音做正式 A/B。

## 已开发(Done)

1. **CosyVoice 2 服务**(:8095)— 零样本声纹克隆 + instruct 情感/语气控制。
   端到端验证:`/tts/zero_shot`、`/tts/instruct` 均 HTTP 200,出 24kHz 音频。
2. **GPT-SoVITS v2Pro 服务**(:8096)— 零样本 + 每人微调(最高保真路线)。
   零样本验证 HTTP 200(32kHz)。
3. **无头微调流水线** `gptsovits_finetune.py` — 给一段目标人音频,自动:
   切片 → FunASR 转写 → 特征(1abc)→ s2/s1 训练 → 产出权重 → 热加载进 :8096。
   已用参考音频跑通整条链路(2 epoch 冒烟,`FT_RC=0`,微调权重出声 200)。
4. **部署工程化**:两镜像已在 GB10 构建(`cosyvoice2:bakeoff`/`gptsovits:bakeoff`);
   底模(5.2G)、微调 ASR 模型(1.2G)、缓存均**持久化到宿主机并挂载**——
   首次正式微调不再等 ~30 分钟下载。与生产 asr/orchestrator **完全隔离**。
5. **仓库整理 + 提交**:`server/tts/` 全量代码 + 部署 runbook(`README.md`)+
   `.gitignore`(屏蔽 234M 构建产物与不入 git 的第三方源码)+ HANDOFF §8 指针。
   提交 `1846ca4`(`feat(tts): ...`)。

## 架构(数据流)

```
参考音频/目标人录音
   ├─(即时)→ CosyVoice2 :8095  ── zero-shot / instruct ──→ 克隆音频
   └─(微调)→ gptsovits_finetune.py ─ 切片/ASR/特征/训练 ─→ 每人权重
                                      └→ 热加载 → GPT-SoVITS :8096 → 高保真音频
两服务均基于 funasr-asr:arm64(GB10 唯一可用 cu130 torch),独立 compose。
```

## 关键决策(为什么这么选)

- **CosyVoice 2**:中文零样本第一梯队、原生情感/语气指令、即时无需训练 → 兜底+情感。
- **GPT-SoVITS v2Pro**:每人微调保真上限最高、社区最成熟的中文克隆 → 「效果最好」路线。
- **基于 funasr-asr:arm64、不装 torch**:GB10(arm64/Blackwell sm_121)只有该基础
  镜像的 `torch 2.13.dev+cu130` 能用;装别的 torch 会废。
- **以已构建镜像为部署载体**:第三方源码不入 git;日常部署不重建、不联网。

## 待办 / 下一步可选项(TODO)

- [ ] **远端 push**:本次执行(`git push -u origin redesign/client-server`)。
- [ ] **正式 A/B(最关键)**:你录 5–30 分钟干净录音 →
      CosyVoice 零样本 vs GPT-SoVITS 微调,盲听定方案。参数建议:微调
      `--epochs-s2 8 --epochs-s1 15`(冒烟用的 2 epoch 不代表音质)。
- [ ] **接入主产品**:选定方案后,把 TTS 接进 orchestrator(新增协议/端点),
      客户端加入口。属较大改动,定方案后单独规划。
- [ ] **第三方源码获取**(仅当要重建 GPT-SoVITS 镜像时):你在能访问 github
      的机器 clean-clone 后 scp 到 GB10 `~/server/tts/GPT-SoVITS/`(README 有步骤)。
- [ ] (可选)第三个对比项:GB10 上已有 `fish-speech-webui:cuda` 镜像
      (Fish Speech / OpenAudio S1),需补权重才能跑;视需要再说。
- [ ] (可选)CosyVoice `fp16` 提速:compose 里 `COSYVOICE_FP16=1`。

## 已知限制 / 注意

- 冒烟微调用的是参考音频 + 2 epoch,**音质不代表正式效果**,只证明流水线通。
- GPT-SoVITS `/tts` 即使用微调权重,**仍需传一个 ref_audio**(架构如此)。
- 国内网络抖动:GB10 构建/下载偶发失败,**重试即可**(Aliyun pip / ModelScope 可靠,
  github 不通——已用 vendor 规避)。
- 容器重建会丢热加载的微调权重(权重文件在 `~/tts-io/gs_train/<exp>/weights` 持久,
  重跑微调或手动 `/set_*_weights` 即可恢复)。

## 怎么自己上手验证

- 现成对比音频:`scp -r fengqi@192.168.0.68:~/tts-io/out ./tts-out`
  (`api_zeroshot.wav`/`api_instruct_happy.wav` = CosyVoice;
   `gs8096_zeroshot.wav` = GPT-SoVITS v2Pro 零样本;
   `gs_finetuned_smoke.wav` = 微调冒烟,仅验证链路)。
- 用自己的声音:见 `README.md` 的「bake-off 使用」两段命令。

## GB10 现场

- 容器:`tts-cosyvoice-1`(:8095)、`tts-gptsovits-1`(:8096)在跑;
  生产 `server-orchestrator-1`/`server-asr-1`/`gemma4-26b-a4b` 未受影响。
- 部署目录:`~/server/tts/`(含 vendor 的 `GPT-SoVITS/` 源码,不在 git)。
- 资产:`~/funasr-prep/models/CosyVoice2-0.5B`、`~/gpt-sovits-assets/`、
  `~/gpt-sovits-cache/`、`~/tts-io/`。
