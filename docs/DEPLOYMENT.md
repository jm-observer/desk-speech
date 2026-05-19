# 部署 / 运维文档(统一)

> 全系统部署与运维的**单一参考**。状态/续作看 `HANDOFF.md`;
> TTS bake-off 细节看 `server/tts/README.md`(本文只做汇总与索引)。

## 1. 拓扑

```
Windows 桌面客户端(Tauri/Rust,采麦+UI+剪贴板,远程模式)
   │  WebSocket  ws://192.168.0.68:8090/stream
   ▼
GB10  192.168.0.68  (NVIDIA GB10 / arm64 / CUDA13 / Ubuntu24 / Docker)
   ├─ orchestrator 容器   :8090  WS 编排 + SQLite + Web 管理台 + /api/*
   ├─ asr 容器            :9100/9101(内部)  FunASR 流式识别 + 声纹门控 + /embed
   ├─ vLLM 主机进程       :8085  gemma-4-26B-A4B-it(润色/翻译)
   └─ TTS bake-off(独立,与上面隔离)
        ├─ tts-cosyvoice-1   :8095  CosyVoice 2(零样本+情感)
        └─ tts-gptsovits-1   :8096  GPT-SoVITS v2Pro(零样本+每人微调)
```

## 2. GB10 接入与目录

- SSH:`ssh fengqi@192.168.0.68`(密钥免密;脚本/命令加 `-o BatchMode=yes`)
- 生产栈目录:`~/server`(`compose.yaml`、`asr/`、`orchestrator/`、`log/`、`smoke_client.py`)
- TTS 栈目录:`~/server/tts`(`compose.tts.yaml`、Dockerfile.*、服务/脚本;
  含不入 git 的 vendor `GPT-SoVITS/`)
- 模型/资产卷(宿主机,不进镜像):
  | 路径 | 用途 |
  |---|---|
  | `~/funasr-prep/models` | 生产 asr 模型(Paraformer/VAD/标点/SenseVoice/CAM++);CosyVoice2 也在此 |
  | `~/gpt-sovits-assets/` | GPT-SoVITS 底模(~5.2G) |
  | `~/gpt-sovits-cache/` | 微调 ASR 模型(~1.2G)+ 容器 `/root/.cache` |
  | `~/tts-io/` | TTS 参考音频/输出/微调数据集与权重 |
  | 命名卷 `orch-data` | orchestrator SQLite `/data/app.db` |

## 3. 端口与卷一览

| 服务 | 端口(宿主) | 对外 | 持久化卷 |
|---|---|---|---|
| orchestrator | 8090 | 是(客户端 WS + Web 台 + /api) | `orch-data:/data` |
| asr | 9100/9101 | 否(内部) | `~/funasr-prep/models:/models:ro` |
| vLLM | 8085 | 主机 | — |
| CosyVoice 2 | 8095 | 是 | `~/funasr-prep/models:/models:ro`, `~/tts-io:/io` |
| GPT-SoVITS | 8096 | 是 | `~/gpt-sovits-assets/*`, `~/gpt-sovits-cache/*`, `~/tts-io:/io` |

## 4. 部署 / 重新部署

### 4.1 生产栈(orchestrator + asr)

```bash
# 改代码后:本地 scp → GB10 重建滚动更新
scp <改动文件> fengqi@192.168.0.68:~/server/<path>
ssh fengqi@192.168.0.68 'cd ~/server && docker compose build <asr|orchestrator> && docker compose up -d'
```
vLLM 是主机进程(不在 compose),维持现状,不随上面重启。

### 4.2 TTS 栈(独立,不影响生产)

```bash
cd ~/server/tts
# 日常部署/重启/掉电恢复 —— 用已构建镜像,不重建、不联网
docker compose -f compose.tts.yaml up -d
# 仅代码/Dockerfile 改动时才重建(GPT-SoVITS 重建需先备好 vendor 源码,见 server/tts/README.md)
docker compose -f compose.tts.yaml build <cosyvoice|gptsovits>
docker compose -f compose.tts.yaml up -d <cosyvoice|gptsovits>
```

### 4.3 构建通用约定

- 构建耗时长 → **后台 detach + 轮询日志**:
  `setsid bash -c "docker compose build x > ~/log/x.log 2>&1; echo RC=\$? >> ~/log/x.log"`
- ⚠️ **国内网络坑**(关键):
  - Docker Hub 基镜像偶发 i/o timeout → 先 `docker pull` 预拉、重试
  - crates 用 `rsproxy.cn`(已在 orchestrator/Dockerfile);pip 用 Aliyun 镜像 + `--no-cache-dir`
  - **github 在 GB10 不通** → GPT-SoVITS 镜像用 vendor 源码 `COPY`(不 clone)
  - 模型走 ModelScope(`~/ms_venv/bin/modelscope`)/可靠;构建抖动**重试即可**
- TTS 服务基于 `funasr-asr:arm64`,**不要装 torch/torchaudio**(GB10 sm_121 只此 torch 可用);
  torchaudio→soundfile 垫片;`numpy<2`/`gradio<5`(GPT-SoVITS)。详见 Dockerfile 注释。

## 5. 桌面客户端(Windows)

```powershell
cd D:\git\streaming-speech
$env:REMOTE_ASR_URL="ws://192.168.0.68:8090/stream"   # 必须;同窗口先设再跑
npm run dev
```
- 远程模式跳过本地模型;录音→说话→**停止后**出结果(P0 按句,无实时逐字)
- 设置页「自动复制」=优化中文 → 自动入剪贴板
- 编译验证:`cd src-tauri && cargo check`;前端 `cd src && npx tsc --noEmit`

## 6. 冒烟检查

```bash
# 生产栈
curl -s http://192.168.0.68:8090/api/stats        # 概览
curl -s http://192.168.0.68:8090/api/asr-config   # asr 运行配置
# Web 管理台:浏览器 http://192.168.0.68:8090/
ssh fengqi@192.168.0.68 'cd ~/server && docker compose logs --tail=40 asr'   # [asr][cfg]/[seg]/[spk]

# TTS
curl -s http://192.168.0.68:8095/health           # CosyVoice
curl -s -o /tmp/t.wav -X POST http://192.168.0.68:8096/tts \
  -H 'Content-Type: application/json' \
  -d '{"text":"测试","text_lang":"zh","ref_audio_path":"/io/ref_sample.wav","prompt_text":"参考","prompt_lang":"zh"}'
```

## 7. 索引

- 状态 / 续作 / 剩余任务:`docs/HANDOFF.md`
- 设计背景:`docs/redesign-architecture-overview.md`、`p0-plan.md`、`protocol-draft.md`、
  `client-refactor-plan.md`、`pronunciation-coach-overview.md`
- TTS bake-off 详解:`server/tts/README.md`(runbook)、`server/tts/STATUS.md`(总览/待办)
- 历史归档:`docs/old/`
