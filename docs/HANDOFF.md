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

关键提交(`git log --oneline`,新到旧):`9c54be1` 实时配置;`5981b9e` 文件上传注册;
`7009450` 声纹门控;`3c588a1` 持久卷;`11c9b6a` 持久化+Web台;`72f7ca3` E 清理;
`6d167ba` D 健壮;P0 系列在更早。

---

## 3. 剩余任务(优先级从高到低)

### Stage3-B:ASR 模型热切换(Paraformer ↔ SenseVoice)
- 加配置键 `asr.model`(`paraformer`|`sensevoice`);`/api/asr-config` 带上它
- asr 轮询检测变化时**重载** `ASR_MODEL`(`AutoModel` 重新构造并替换全局;
  SenseVoiceSmall 已在 `~/funasr-prep/models/SenseVoiceSmall`,加 env `ASR_SENSEVOICE_DIR`)
- 管理台「配置」加下拉(或直接编辑该键即可,已是通用 kv 编辑)
- 注意:重载需在不打断进行中会话时做(简单起见:下次会话生效,或加锁)

### Stage3-C:LLM 配置移入管理台
- orchestrator seed 默认键:`vllm.model`、`vllm.base`、`llm.optimize_prompt`、`llm.translate_prompt`
- `asr_reader`/`llm()` 改为优先读 DB config(回退 env/常量);`asr_reader` 已持有 `db`,
  直接 `db.config_get(...)`;模型/base 注入 `llm()`(现签名 `llm(&Cfg,sys,user)`,
  可加 model/base 参数或临时 Cfg)
- 提示词当前硬编码在 `server/orchestrator/src/main.rs` `asr_reader` 内(优化/翻译两段)

### C:识别质量(用户最初定为最后做)
- Paraformer vs SenseVoice A/B(靠 Stage3-B 切换后对比);提示词调优
- 已知:Paraformer-large 中文好;turbo/sherpa 已弃用

### 暂缓:声纹注册端到端实测
- 浏览器麦克风需 HTTPS/localhost;明文 http LAN 下 `getUserMedia` 不可用
- 现已提供「上传音频注册」(任意格式,asr ffmpeg 解码)——用音频文件即可测
- 可选改进:给 orchestrator 加自签 HTTPS 以支持浏览器实时录音注册
- 阈值 `ASR_SPK_THRESHOLD` 默认 0.35(CAM++ 余弦),经管理台配置页调

### 杂项(可选)
- 客户端 ~27 条 dead-code 警告(E 删 legacy 残留),可清
- `protocol-draft` 预留的 `speaker` 字段透传到桌面 UI 显示说话人(现仅入库/历史可见)
- `protocol.rs` 的 `Hello` 未读字段告警

---

## 4. 部署 / 运维速查(GB10)

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
4. 从 **Stage3-B** 开始(或按用户当时指示);按"改→本地 cargo check→scp→compose build/up→冒烟→commit"节奏,逐片提交
