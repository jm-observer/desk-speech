# 交接 / 续作文档(自包含)

> 新 session 从这里接手。本文不依赖对话记忆,读完即可继续开发。
> 分支:**`redesign/client-server`**(所有重构工作在此分支)。

---

## 1. 这个项目现在是什么

原 StreamSpeech(单体 Tauri 离线语音转写)已**重构为瘦客户端 + GB10 服务端**:

```
Windows 桌面端(Tauri/Rust,仅采麦+UI+剪贴板,远程模式)
   │  WebSocket 协议(docs/protocol-draft.md):上行16k PCM,下行 segment/optimized/translated
   ▼
GB10(192.168.0.68, NVIDIA GB10 / arm64 / CUDA13, Ubuntu24, Docker)
   ├─ orchestrator 容器(Rust/axum):WS 编排 + SQLite 持久化 + Web 管理台 + HTTP API
   ├─ asr 容器(Python/FunASR):流式 VAD + Paraformer 识别 + 标点 + 声纹门控 + /embed
   └─ vLLM(主机进程,非容器):gemma-4-26B-A4B-it,润色/翻译,端口 8085
```

设计决策与背景见:`docs/redesign-architecture-overview.md`、`p0-plan.md`、
`protocol-draft.md`、`client-refactor-plan.md`、`pronunciation-coach-overview.md`。

---

## 2. 已完成(均已提交到 `redesign/client-server`)

- P0 远程闭环:采麦→GB10 识别→优化/翻译→回桌面 UI;剪贴板自动复制
- 流式 VAD **按句**分段(asr 自定义断句,阈值可调);卡片完整(原文+优化+英文同卡,upsert 不丢);时间 HH:MM:SS
- D:断线自动重连 + 连接失败界面"异常"提示
- E:客户端**剥离 sherpa-onnx/GPU/模型打包**(纯远程瘦客户端,无 2.6GB/DLL/NSIS 之痛)
- Web 管理台 + SQLite 持久化:概览(录音时长统计)/历史/声纹/配置
- 多声纹门控:asr 加载 CAM++ zh+en 模型,每句算 embedding,与"已启用声纹"余弦比对,
  命中才识别(带说话人名),否则丢弃;未注册→门控自动关。注册经管理台上传音频→
  orchestrator `/api/speakers/enroll`→asr `/embed`→存库
- Stage3-A:管理台「配置」页可实时调 `asr.spk_threshold` / `asr.sentence_gap_ms`
  (asr ~15s 轮询 `/api/asr-config` 生效,免重部署)
- Stage3-B:ASR 模型热切换。配置键 `asr.model`(`paraformer`|`sensevoice`,
  已 seed,入 `/api/asr-config`)。asr 轮询检测变化→在 worker 线程重建识别器
  并原子替换全局;`recognize()` 快照 model+kind,进行中会话用旧模型收尾不中断。
  SenseVoice 输出经 `rich_transcription_postprocess` 去富标签。已实测双向切换
  (无重启、两侧端到端识别均正常)。配置键直接在「配置」页通用 kv 编辑即可改
- Stage3-C:LLM 配置移入管理台。seed `vllm.model`/`vllm.base`/`llm.optimize_prompt`/
  `llm.translate_prompt`(默认取 env/常量)。`asr_reader` 每段从 DB 读这 4 键
  (回退 env/常量);`llm()` 签名改为 `llm(base,model,sys,user)`。提示词/模型/base
  在「配置」页实时可编,已实测改提示词**免重部署即时生效**、可还原
- C(识别质量)A/B 已做:`server/ab_asr.py`(容器内裸跑两模型对比)。结论见 §3。
  代码侧落地:SenseVoice 改为正则剥 `<|...|>` 标签(弃 `rich_transcription_postprocess`,
  它会把 `<|BGM|>`/`<|ANGRY|>` 注成 🎼/😡 噪声),正文已自带 ITN 标点
- 逐段音频留存(commit `f9a6d7d`):orchestrator 缓存上行 PCM,asr 定段时按
  [t0,t1] 切片存 WAV(`segment_audio` 表),**asr/协议零改动**。留 1 天;管理台
  「历史」每行 ▶试听 / ⬇下载(可作声纹注册输入)/ 改原文+保存(生成纠错样本)。
  端点 `GET|POST /api/segments/:id/audio|text`;每小时清理过期音频,文本保留。
  已端到端实测(切片字节数精确、改文持久、回填过期后被清且文本留存)。
  会话 PCM 缓冲已改 `PcmBuf`(~180s 上限,带 base 偏移),长会话不再无限涨内存
- 客户端深色重做 + 连接错误可视化(commit `6825614`):主题翻深色对齐管理台;
  启动错误/异步连接失败明确提示原因 + 重试按钮(不再只一个"异常");**不再
  预载本地旧库历史**(开机空,本会话经 `segment_updated` 实时填;历史看 :8090);
  录音按钮收敛(去掉报警式大红圆);去掉 RecordCard 重复状态块
- SEG_ID 重启撞 id **已修**(commit `5a58436`):启动 `SEG_ID=MAX(id)+1`,
  重启不再覆盖旧 segment/音频。实测 max=11→新段 id=12,旧行完好
- LLM 并发(commit `5a58436`):段事件立即转发,optimize/translate 每段独立
  task 内 `tokio::join!` 并发;段 N+1 不被 N 的 LLM 阻塞;Done 前 drain-await
  在飞任务(乱序按 ref id 归并安全)
- 日志降噪(`838e5bc`)、文案/lint 小修(`2144ba8`)、PCM限长(`bb80d0b`)
- 声纹门控开关(commit `c5046bf`):新 config 键 `asr.gate_to_enrolled`
  (`on`|`off`,seed `on`,入 `/api/asr-config`,asr ~15s 轮询)。on=仅识别
  已启用声纹(其余丢弃);off=识别所有人(命中仍标说话人)。管理台「声纹」
  页加复选框开关。已端到端实测(异己 clip:on 丢弃 score0.016 / off 识别 spk=None)

关键提交(`git log --oneline`,新到旧):`c5046bf` 门控开关;`341cd8d`/`bb80d0b` PCM限长;
`2144ba8` 文案/lint;`838e5bc` 日志降噪;
`5a58436` SEG_ID修复+LLM并发;`6825614` 深色重做+错误UX;`f9a6d7d` 音频留存;`acc253b` orch零警告;
`b626f2e` speaker入UI;`859036c` spk_embed修复;`dbd74e0` 客户端死码清理;
`e5861fa` SV去标签;`ddd8e15` LLM 配置入台;`18d4db5` ASR 热切换;`9c54be1` 实时配置;`5981b9e` 文件上传注册;
`7009450` 声纹门控;`3c588a1` 持久卷;`11c9b6a` 持久化+Web台;`72f7ca3` E 清理;
`6d167ba` D 健壮;P0 系列在更早。

---

## 3. 剩余任务(优先级从高到低)

### C:识别质量 —— A/B 已完成,结论如下(剩提示词微调,可选)
A/B 工具:`server/ab_asr.py`(scp 到 `~/server`→`docker compose cp` 进 asr→
`docker compose exec -T asr python3 /tmp/ab_asr.py`)。基于 bundled zh/yue/en 片段:
- **Paraformer-large**:纯普通话内容词略稳(如 "开放时间" vs SV 误 "开饭时间");
  但**裸输出无标点**(本项目靠 punc/LLM 优化补),且**仅中文**(粤/英严重乱码),慢 ~5-25x
- **SenseVoice**:快很多、原生标点、ITN 阿拉伯数字、多语(粤/英/日/韩 OK);
  偶发个别内容词错。原 emoji 噪声已在代码侧修掉
- **建议**:默认保持 `paraformer`(中文最稳,输出过 LLM 优化补标点);需低延迟/
  多语场景一键切 `sensevoice`(管理台「配置」`asr.model`)。两者用户都可用自己
  声音在管理台 A/B
- 提示词调优:当前优化/翻译提示词够用,无具体抱怨不盲改;`llm.optimize_prompt`/
  `llm.translate_prompt` 现已是管理台实时可编(Stage3-C),按需现场调即可

### 声纹注册端到端 —— 已打通(原"暂缓"已解决)
- 修了潜伏 bug:`spk_embed` 把 CAM++ 的 cuda 张量直接 `np.asarray` 会崩
  (`can't convert cuda:0 tensor to numpy`)——这正是当初"暂缓"的原因。已加
  `.detach().cpu()`(commit `859036c`)。实测:上传音频注册→`/embed`→存库→
  门控对"重说同一人"命中,段带 speaker
- 浏览器麦克风仍需 HTTPS/localhost;明文 http LAN 用「上传音频注册」即可(已验证)
- 阈值 `ASR_SPK_THRESHOLD` 默认 0.35(CAM++ 余弦),经管理台配置页调
- 可选改进(未做):给 orchestrator 加自签 HTTPS 以支持浏览器实时录音注册

### 杂项
- ~~客户端 dead-code 警告~~ **已清**(commit `dbd74e0`):删 db_worker/判定子系统/
  CorrectionEngine.apply/push_samples/lib.rs 死结构等;`cargo check` 24→0 警告,
  `cargo test` 70 通过(删了只测已删功能的 legacy 测试文件)
- ~~`speaker` 字段透传桌面 UI~~ **已做**(commit `b626f2e`):协议加可选 speaker→
  orchestrator 填→remote.rs 透传→store→SegmentCard 显示说话人 chip(新增 user 图标);
  端到端实测段事件含 `"speaker":"测试说话人"`
- ~~orchestrator dead_code 警告~~ **已清**(commit `acc253b`):删 `ServerEvent::Status`,
  `Hello` 加 `#[allow(dead_code)]`(协议契约字段);`cargo check` 0 警告
- ~~SEG_ID 重启撞 id~~ **已修**(commit `5a58436`):启动时
  `SEG_ID=MAX(segments.id)+1`,重启不再覆盖旧 segment/音频(实测 §2)

---

## 4. 部署 / 运维速查(GB10)

> 完整版(全栈拓扑/端口卷表/重建流程/客户端/冒烟)见 **`docs/DEPLOYMENT.md`**。下面是速查。

- SSH:`ssh fengqi@192.168.0.68`(密钥免密;命令用 `-o BatchMode=yes`)
- 服务目录:`~/server`(`compose.yaml`、`asr/`、`orchestrator/`、`log/`、`smoke_client.py`)
- 模型卷:`~/funasr-prep/models` → 容器 `/models:ro`(Paraformer/VAD/标点/SenseVoiceSmall/
  CAM++ `speech_campplus_sv_zh_en_16k-common_advanced` 均已下载)
- 持久化:命名卷 `orch-data` → `/data/app.db`(sessions/segments/speakers/config)
- 端口:**8090** orchestrator(对客户端 WS `/stream` + Web 台 `/` + `/api/*`,对外可达);
  asr 9100(ws)/9101(/embed)内部;vLLM **8085** 主机(`gemma-4-26B-A4B-it`)
- 改代码后部署:本地 `scp` 改动文件到 `~/server/...` → `docker compose build [svc] && docker compose up -d`
- **构建用后台 detach + 轮询日志**(见历史做法:`setsid bash -c "... ; echo RC=$? >> log/x.log"`)
- ⚠️ 网络坑(国内):Docker Hub 拉 `rust:1-bookworm`/`debian:bookworm-slim` 偶发 i/o timeout
  → 先 `docker pull` 预拉重试;crates 用 `rsproxy.cn` 镜像(已在 orchestrator/Dockerfile),
  偶发抖动**重试构建**即可;ModelScope 下模型用 `~/ms_venv/bin/modelscope`
- 冒烟:`curl http://localhost:8090/api/stats|/api/asr-config|/api/voiceprints`;
  `docker compose logs --tail=N asr`(关注 `[asr][cfg]`/`[asr][seg]`/`[asr][spk]`)
- Web 管理台:浏览器开 `http://192.168.0.68:8090/`

---

## 5. 桌面客户端怎么跑(Windows)

```powershell
cd D:\git\streaming-speech
$env:REMOTE_ASR_URL="ws://192.168.0.68:8090/stream"   # 必须;同窗口先设再跑
npm run dev
```
- 远程模式跳过本地模型,启动快;点开始录音→说话→**停止后**出结果(P0 无实时逐字,
  按句:停顿 > `asr.sentence_gap_ms` 才切句)
- 设置页「自动复制」=优化中文 → 优化结果自动入剪贴板
- 编译验证:`cd src-tauri && cargo check`;前端 `cd src && npx tsc --noEmit`

---

## 6. 关键文件地图

- 客户端:`src-tauri/src/commands/remote.rs`(远程会话/重连/剪贴板/重采样)、
  `commands/recording.rs`(采麦+start/stop/clear/state,远程only)、`lib.rs`(瘦身后)、
  `src/src/store/useAppStore.ts`(segment_updated upsert)、`components/SettingsModal.tsx`
- 服务端:`server/asr/app.py`(VAD/识别/断句/声纹门控/`/embed`/配置轮询)、`server/asr/Dockerfile`
  (FROM `funasr-asr:arm64` 薄层)、`server/orchestrator/src/main.rs`(WS编排+HTTP API+内嵌Web台
  `CONSOLE_HTML`)、`src/db.rs`(rusqlite)、`src/protocol.rs`、`server/compose.yaml`
- 文档:`docs/*.md`(本文 + 架构/协议/计划)
- 记忆:`C:\Users\36225\.claude\projects\D--git-streaming-speech\memory\project_gpu_build.md`
  (GB10/CUDA 构建坑;客户端已不再 GPU,但 asr/orchestrator 构建坑仍参考)

---

## 7. 新 session 第一步建议

1. `git -C D:/git/streaming-speech branch --show-current` 确认在 `redesign/client-server`
2. 读本文 + `docs/redesign-architecture-overview.md` §9(已定决策)
3. `ssh -o BatchMode=yes fengqi@192.168.0.68 'cd ~/server && docker compose ps && curl -s localhost:8090/api/stats'`
   确认服务在跑
4. P0 + Stage3-A/B/C + 质量 C **均已完成**。剩余只有「暂缓:声纹注册端到端实测」
   和「杂项(可选)」——按用户当时指示挑选;按"改→本地 cargo check→scp→
   compose build/up→冒烟→commit"节奏,逐片提交

---

## 8. TTS bake-off(新增能力,独立于主链路)

为给项目选 TTS 方案而搭的两个**独立服务**(与生产 asr/orchestrator 隔离):
**CosyVoice 2 :8095**(零样本+情感)与 **GPT-SoVITS v2Pro :8096**(零样本+每人微调)。
两镜像已在 GB10 构建;日常用已构建镜像 `docker compose -f compose.tts.yaml up -d`。

- **单一文档:`server/tts/README.md`**(拓扑/端口/卷、日常部署、重建流程+GB10 坑、
  微调用法、文件清单)。续作 TTS 先读它。
- vendor 的 `server/tts/GPT-SoVITS/` 第三方源码**不入 git**(`.gitignore` 已配),
  仅重建镜像时需要;日常部署不需要。
- 记忆:`memory/project_tts_bakeoff.md`(为何做、已搭什么、如何续作)。
