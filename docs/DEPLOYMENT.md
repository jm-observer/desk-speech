# 部署 / 运维文档(统一)

> 全系统部署与运维的**单一参考**。状态/续作看 `HANDOFF.md`;
> TTS bake-off 细节看 `server/tts/README.md`(本文只做汇总与索引)。

> ⚠️ **asr-server 已物理退役**。先从本仓迁至 toolkit 仓 `crates/asr-server`（2026-06 早），
> 后因与本仓 FunASR 能力重叠在 toolkit 又被整 crate 删除（2026-06 中后期）。
> 外部离线转写**统一改走本仓 FunASR 的 `/transcribe` 端点**（`server/asr` :9101，
> multipart 上传 wav/mp3/mp4）。文中所有 `:8091` / `~/asr-server-models` / sherpa profile
> 的条目均为历史记录，**不再代表当前部署**。详见 `server/asr-server/MOVED.md`。

## 1. 拓扑

```
Windows 桌面客户端(Tauri/Rust,采麦+UI+剪贴板,远程模式)
   │  WebSocket  ws://192.168.0.68:8090/stream
   ▼
GB10  192.168.0.68  (NVIDIA GB10 / arm64 / CUDA13 / Ubuntu24 / Docker)
   ├─ orchestrator 容器   :8090  WS 编排 + SQLite + Web 管理台 + /api/*
   ├─ asr 容器            :9100(内部 WS) | 127.0.0.1:9101(HTTP)
   │                      FunASR 流式识别 + 声纹门控 + /embed + /transcribe
   ├─ vLLM 主机进程       :8085  gemma-4-26B-A4B-it(润色/翻译)
   └─ TTS(独立,与上面隔离;按需启动)
        └─ cosyvoice       :8095  CosyVoice 2(bake-off 胜出;GPT-SoVITS
                                  2026-05-28 弃用,已从 compose 移除)

外部消费方(同机 toolkit 抖音管线): http://127.0.0.1:9101/transcribe
  multipart audio=<mp4 bytes> [vad=1|0]
  → {text, segments:[{t_start,t_end,text}], model}
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
| asr | 9100(WS,容器内) / 127.0.0.1:9101(HTTP `/embed`+`/transcribe`) | 9100 内部 / 9101 仅本机 | `~/funasr-prep/models:/models:ro` |
| vLLM | 8085 | 主机 | — |
| ~~asr-server~~ | ~~8091~~ | 已物理退役(2026-06) | — |
| CosyVoice 2 | 8095(按需启动) | 是 | `~/funasr-prep/models:/models:ro`, `~/tts-io:/io`, `~/tts-voices:/voices:ro` |
| ~~GPT-SoVITS~~ | ~~8096~~ | 已弃用(2026-05-28) | 卷与归档见 `server/tts/STATUS.md` |

## 4. 数据存储(SQLite)

orchestrator 的所有持久状态都在一个 SQLite 文件里:容器内 `/data/app.db`,
挂载到 Docker volume `server_orch-data`(宿主机 `/var/lib/docker/volumes/server_orch-data/_data/app.db`)。
不需要任何外部数据库依赖;`rusqlite` bundled,跟 orchestrator 二进制一起编进镜像。

| 表 | 用途 | 何时清理 |
|---|---|---|
| `segments` | 识别原文 / 优化 / 翻译 / 说话人 / `t_start`/`t_end` / 时间戳 | 永久。管理台「历史」可单条删除或一键「清空全部历史」 |
| `segment_audio` | 每段音频的 WAV blob | **orchestrator 每小时自动清 1 天前的 blob**(`audio_purge_expired`);清空历史时同步删除 |
| `sessions` | 录音会话(起止时间、累计时长) | 永久。即使清空历史也保留 → 概览 tab 的"录音时长/今日"统计不会丢 |
| `speakers` | 声纹库(`name` + embedding CSV + `enabled`) | 永久。管理台「声纹」tab 管理 |
| `config` | 运行时 kv:`asr.model` / `asr.spk_threshold` / `asr.sentence_gap_ms` / `asr.gate_to_enrolled` / `vllm.base` / `vllm.model` / `llm.optimize_prompt` / `llm.translate_prompt` 等 | 永久。管理台「配置」tab 实时可编;asr 容器每 ~15s 轮询热更新 `asr.*`,`vllm.*` / `llm.*` 在每条新分段处理时即时读取 |

### 4.1 备份 / 恢复

```bash
# 备份(随时,不停服):
ssh fengqi@192.168.0.68 'docker cp server-orchestrator-1:/data/app.db /tmp/orch-backup.db'
scp fengqi@192.168.0.68:/tmp/orch-backup.db ./orch-backup-$(date +%F).db

# 恢复(会覆盖现有数据,先停 orchestrator):
ssh fengqi@192.168.0.68 'docker compose -f ~/server/compose.yaml stop orchestrator'
scp orch-backup.db fengqi@192.168.0.68:/tmp/
ssh fengqi@192.168.0.68 'docker cp /tmp/orch-backup.db server-orchestrator-1:/data/app.db \
    && docker compose -f ~/server/compose.yaml start orchestrator'
```

### 4.2 彻底重置(慎用)

```bash
docker compose -f ~/server/compose.yaml down
docker volume rm server_orch-data
docker compose -f ~/server/compose.yaml up -d
```

会**全部丢失**:识别历史 / 声纹注册 / 运行时配置。重启会用 env 变量重新 seed 默认配置
(`compose.yaml` 里的 `VLLM_BASE` / `VLLM_MODEL`、`orchestrator/src/main.rs:56-59` 里的提示词常量)。

### 4.3 临时检索 / 调试

容器里**没装 `sqlite3` CLI**(镜像精简过)。要看 DB 内容,拷出来再查:

```bash
ssh fengqi@192.168.0.68 'docker cp server-orchestrator-1:/data/app.db /tmp/peek.db'
ssh fengqi@192.168.0.68 'sqlite3 /tmp/peek.db ".tables"'
ssh fengqi@192.168.0.68 'sqlite3 /tmp/peek.db "SELECT key,value FROM config"'
```

或者整个表导出 CSV:
```bash
ssh fengqi@192.168.0.68 'sqlite3 -header -csv /tmp/peek.db \
    "SELECT id,ts,text,optimized,english,speaker FROM segments ORDER BY id DESC LIMIT 100"' > peek.csv
```

日常运维不需要这一层 —— 管理台 `http://192.168.0.68:8090/` 的「配置」/「历史」/「声纹」三个 tab
覆盖了 99% 操作,API 也有 `GET /api/history?limit=N`、`GET /api/segments/:id`、
`POST /api/segments/:id/rerun`、`DELETE /api/segments[/:id]`、`POST /api/config` 等。

## 5. 部署 / 重新部署

### 5.1 生产栈(orchestrator + asr)

```bash
# 改代码后:本地 scp → GB10 重建滚动更新
scp <改动文件> fengqi@192.168.0.68:~/server/<path>
ssh fengqi@192.168.0.68 'cd ~/server && docker compose build <asr|orchestrator> && docker compose up -d'
```
vLLM 是主机进程(不在 compose),维持现状,不随上面重启。

### 5.2 TTS 栈(独立,不影响生产)

```bash
# 同步改动文件(发布脚本不覆盖 tts,手动 scp)
scp server/tts/<改动文件> fengqi@192.168.0.68:~/server/tts/
# GB10 上:
cd ~/server/tts
# 日常启动/掉电恢复 —— 用已构建镜像,不重建、不联网
docker compose -f compose.tts.yaml up -d
# 仅代码/Dockerfile 改动时才重建(cosy_server.py 在末层,缓存命中下秒级)
docker compose -f compose.tts.yaml build cosyvoice
```
容器**按需启动**(不用时 stop 省 GPU);启动即预热(权重 + dummy 合成,
`/health` 可达即就绪,详见 `server/tts/API.md`)。

### 5.3 构建通用约定

- 构建耗时长 → **后台 detach + 轮询日志**:
  `setsid bash -c "docker compose build x > ~/log/x.log 2>&1; echo RC=\$? >> ~/log/x.log"`
- ⚠️ **国内网络坑**(关键):
  - Docker Hub 基镜像偶发 i/o timeout → 先 `docker pull` 预拉、重试
  - crates 用 `rsproxy.cn`(已在 orchestrator/Dockerfile);pip 用 Aliyun 镜像 + `--no-cache-dir`
  - **github 在 GB10 不通** → GPT-SoVITS 镜像用 vendor 源码 `COPY`(不 clone)
  - 模型走 ModelScope(`~/ms_venv/bin/modelscope`)/可靠;构建抖动**重试即可**
- TTS 服务基于 `funasr-asr:arm64`,**不要装 torch/torchaudio**(GB10 sm_121 只此 torch 可用);
  torchaudio→soundfile 垫片;`numpy<2`/`gradio<5`(GPT-SoVITS)。详见 Dockerfile 注释。

## 6. 桌面客户端(Windows)

```powershell
cd D:\git\streaming-speech
npm run dev
```
- 远程模式跳过本地模型;录音→说话→**停止后**出结果(P0 按句,无实时逐字)
- 连接地址在控制面板「连接地址」下拉里选/加(内置默认 `ws://192.168.0.68:8090/stream`,
  用户可加自定义,持久化在本地 SQLite `remote.url` / `remote.url_presets`);
  录音中切换地址会自动停-启,无需手动重连
- 设置页「自动复制」=优化中文 → 自动入剪贴板
- 编译验证:`cd src-tauri && cargo check`;前端 `cd src && npx tsc --noEmit`

## 7. 冒烟检查

```bash
# 生产栈
curl -s http://192.168.0.68:8090/api/stats        # 概览
curl -s http://192.168.0.68:8090/api/asr-config   # asr 运行配置
# Web 管理台:浏览器 http://192.168.0.68:8090/
ssh fengqi@192.168.0.68 'cd ~/server && docker compose logs --tail=40 asr'   # [asr][cfg]/[seg]/[spk]

# FunASR /transcribe(仅 GB10 本机可达;给同机 toolkit 抖音管线消费)
# 用任意短 wav/mp3/mp4 文件冒烟:
ssh fengqi@192.168.0.68 'curl -s -F audio=@/tmp/smoke.wav -F vad=1 http://127.0.0.1:9101/transcribe'

# TTS(容器启动后)
curl -s http://192.168.0.68:8095/health           # CosyVoice;调用示例见 server/tts/API.md
```

## 8. 索引

- 状态 / 续作 / 剩余任务:`docs/HANDOFF.md`
- 设计背景:`docs/redesign-architecture-overview.md`、`p0-plan.md`、`protocol-draft.md`、
  `client-refactor-plan.md`、`pronunciation-coach-overview.md`
- TTS bake-off 详解:`server/tts/README.md`(runbook)、`server/tts/STATUS.md`(总览/待办)
- 历史归档:`docs/old/`
