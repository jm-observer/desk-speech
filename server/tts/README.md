# server/tts/ — CosyVoice2 TTS 服务

跑在 GB10 上的独立 TTS HTTP 服务。任何项目通过 `http://192.168.0.68:8095` 调用即可。
**与生产 `asr`/`orchestrator` 完全隔离**(独立 compose、独立镜像、独立端口)。

> **调用方看这里**:[**API.md**](API.md)(完整 endpoint 手册 + 示例)
> 部署/运维看下面 ↓

## 拓扑

| 服务 | 容器 | 宿主端口 | 引擎 | 镜像 |
|---|---|---|---|---|
| CosyVoice 2 | `tts-cosyvoice-1` | **8095** | CosyVoice2-0.5B | `cosyvoice2:bakeoff` |

> 历史:之前曾跑 GPT-SoVITS v2Pro(`:8096`)做 A/B,2026-05-28 选定 CosyVoice2 后**已从 compose 移除**。微调流水线/Dockerfile/A/B 脚本已归档到 [`legacy/`](legacy/README.md),保留以供回溯/参考,不再编排部署。

## 宿主机资产(持久化,不进镜像/不进 git)

| 路径 | 内容 | 来源 |
|---|---|---|
| `~/funasr-prep/models/CosyVoice2-0.5B` | CosyVoice2 模型权重(~5GB) | modelscope `iic/CosyVoice2-0.5B` |
| `~/tts-voices/` | **音色库**:`voices.json` + `*.wav` ref 文件 | 见下方"音色库" |
| `~/tts-io/` | 调用方调试时的输入/输出落盘目录(容器内 `/io`) | 运行期 |

## 音色库(`~/tts-voices/`)

```
~/tts-voices/
├── voices.json         # manifest(prompt_text + 每个音色的 id/file/gender/tone/license)
├── edge_xiaoxiao.wav   # ↓ 5 个 Edge TTS 生成的 ref ⚠️ License 仅 dev
├── edge_xiaoyi.wav
├── edge_yunxi.wav
├── edge_yunjian.wav
├── edge_yunyang.wav
└── cosy_zero_shot.wav  # CosyVoice2 官方 ref,Apache-2.0 ✅
```

- 容器以 `:ro` 挂载到 `/voices`,服务通过 `VOICES_DIR=/voices` 读取
- **热更新**:`GET /voices` 每次请求重读 manifest。加新音色:把 wav 丢进去 + 编辑 voices.json 加一条 entry,**不用重启容器**
- ⚠️ **License 风险**:`edge_*` 是 Microsoft Edge TTS 输出,ToS 仅授权个人/开发用。上线前必须替换成 AISHELL-3(CC-BY)或 Common Voice(CC-0),见 [STATUS.md](STATUS.md) 的"License 替换"待办

## 日常部署(用已构建镜像)

```bash
# GB10 ~/server/tts(本仓库 server/tts/ scp 而来)
cd ~/server/tts
docker compose -f compose.tts.yaml up -d
docker compose -f compose.tts.yaml ps
docker compose -f compose.tts.yaml logs --tail=20 cosyvoice
```

`up -d` 仅在镜像缺失时才构建。设置了 `restart: unless-stopped`,GB10 重启会自动拉回。

## 代码改动时重建

仅当改了 `cosy_server.py` / `Dockerfile.cosyvoice` / 升级模型时:

```bash
# 1. 本地改 server/tts/,scp 到 GB10
scp server/tts/cosy_server.py fengqi@192.168.0.68:~/server/tts/

# 2. 在 GB10 重建并热替换
ssh fengqi@192.168.0.68 'cd ~/server/tts
  docker compose -f compose.tts.yaml build cosyvoice
  docker compose -f compose.tts.yaml up -d cosyvoice'

# 3. 冒烟(应返回 model_loaded:true 后再算稳定)
curl -s http://192.168.0.68:8095/health
curl -s -X POST http://192.168.0.68:8095/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"冒烟测试","voice_id":"edge_yunjian"}' -o /tmp/smoke.wav
```

镜像构建 ~10-15 分钟(arm64+CUDA13 全栈)。**急用紧急修复**时可以走"docker cp 热替换 + 重启"
路径(不重建镜像,但下次 `up -d` recreate 时会回到镜像里的旧代码),见 git log 找历史命令。

## GB10 构建坑(已写进 Dockerfile 注释)

- 必须基于 `funasr-asr:arm64`,**不要装 torch/torchaudio**:只有该基础镜像的
  `torch 2.13.dev+cu130` 支持 GB10(sm_121)
- 该 torchaudio 无 soundfile 后端、arm64 无 torchcodec → `cosy_server.py` 启动时把
  `torchaudio.load/save` monkeypatch 到 soundfile
- 当前 CosyVoice 主线 `prompt_wav` 接收 **路径**(而非预加载张量)—— 上传 wav 先 spool 到临时文件再调用
- pip 走 Aliyun 镜像(`pypi.org`/Tsinghua 在该机 SSL 抖动);`--no-cache-dir` 避免坏缓存

## 文件清单

| 文件 / 目录 | 作用 | 在 git? |
|---|---|---|
| [API.md](API.md) | **调用方手册** —— endpoints + 示例 + voice_id | ✅ |
| [STATUS.md](STATUS.md) | 项目状态、待办、License 风险追踪 | ✅ |
| [instruct_prompts.json](instruct_prompts.json) | 实测有效的 instruct enum(产品 UI 数据源) | ✅ |
| `compose.tts.yaml` | 服务编排(端口/挂载/restart 策略) | ✅ |
| `Dockerfile.cosyvoice` | CosyVoice2 镜像(基于 funasr-asr:arm64) | ✅ |
| `cosy_server.py` | HTTP 服务(/health /voices /tts /tts/{zero_shot,instruct,cross_lingual}) | ✅ |
| [`tts-voices/`](tts-voices/README.md) | 本地音色/bake-off 工作区(refs + gen/syn 脚本 + 句子集) | ✅(`outputs/` 除外) |
| [`legacy/`](legacy/README.md) | GPT-SoVITS bake-off 残留(微调流水线、A/B 脚本、Dockerfile) | ✅ |
| `GPT-SoVITS/` | GPT-SoVITS 第三方源码(仅重建 sovits 镜像时需要) | ❌(`.gitignore`) |
