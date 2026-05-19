# server/tts/ — TTS bake-off(CosyVoice 2 vs GPT-SoVITS)

为给项目挑选 TTS 方案(自定义声纹 + 可配置语气 + 接近真人)而搭的**两个独立服务**,
跑在 GB10 上,**与生产 `asr`/`orchestrator` 完全隔离**(独立 compose、独立镜像、独立端口)。

## 拓扑

| 服务 | 容器 | 宿主端口 | 能力 | 镜像 |
|---|---|---|---|---|
| CosyVoice 2 | `tts-cosyvoice-1` | **8095** | 零样本克隆 + instruct 情感/语气 | `cosyvoice2:bakeoff` |
| GPT-SoVITS v2Pro | `tts-gptsovits-1` | **8096** | 零样本 + 每人微调(最高保真) | `gptsovits:bakeoff` |

两镜像均**已在 GB10 构建完成**。日常使用/重启**直接用已构建镜像**,无需重建。

## 宿主机资产(持久化,不进镜像/不进 git)

| 路径 | 内容 | 来源 |
|---|---|---|
| `~/funasr-prep/models/CosyVoice2-0.5B` | CosyVoice2 模型 | modelscope `iic/CosyVoice2-0.5B` |
| `~/gpt-sovits-assets/{pretrained_models,G2PWModel,nltk_data}` | GPT-SoVITS 底模(~5.2G) | modelscope `XXXXRT/GPT-SoVITS-Pretrained` |
| `~/gpt-sovits-cache/asr_models` | 微调用 FunASR paraformer/punc/vad(~1.2G) | 首次微调已下载并固化 |
| `~/gpt-sovits-cache/dot_cache` | 容器 `/root/.cache` 持久化 | 运行期生成 |
| `~/tts-io` | 参考音频 / 输出 / 微调数据集与权重(容器内 `/io`) | 运行期 |

## 日常部署(用已构建镜像 —— 主路径)

```bash
# 部署目录:GB10 ~/server/tts(由本仓库 server/tts/ scp 而来)
cd ~/server/tts
docker compose -f compose.tts.yaml up -d            # 用现有镜像,不重建
docker compose -f compose.tts.yaml ps
docker compose -f compose.tts.yaml logs --tail=20 cosyvoice gptsovits
```
`up -d` 仅在镜像缺失时才构建;已存在镜像不会被重建。重启/掉电恢复同样用这条。

## 后续新版本部署流程(代码改动时才重建)

仅当改了 `cosy_server.py` / `gptsovits_finetune.py` / `Dockerfile.*` / 升级模型时:

1. 本地改 `server/tts/`,提交。
2. scp 到 GB10:`scp server/tts/<改动文件> fengqi@192.168.0.68:~/server/tts/`
3. 在 GB10 重建并滚动更新(后台 + 轮询日志,网络抖动重试即可):
   ```bash
   cd ~/server/tts
   docker compose -f compose.tts.yaml build <cosyvoice|gptsovits>
   docker compose -f compose.tts.yaml up -d <cosyvoice|gptsovits>
   ```
4. 冒烟:`curl -s -m240 -o /tmp/t.wav ... :8095/tts/zero_shot` 或 `:8096/tts`(见下)。

### 重建 GPT-SoVITS 的前置条件(重要)

`Dockerfile.gptsovits` 用 `COPY GPT-SoVITS /app/GPT-SoVITS`(**不 git clone**:GB10
访问不了 github)。因此**重建前**构建上下文 `~/server/tts/` 下必须有 `GPT-SoVITS/`
源码目录(纯代码,排除 `GPT_SoVITS/pretrained_models`、`GPT_SoVITS/text/G2PWModel`、
`.git`、各 `*.zip`)。该目录**不入 git**(见 `.gitignore`),需自行准备:
能访问 github 的机器 `git clone --depth 1 https://github.com/RVC-Boss/GPT-SoVITS`
→ 删上述大目录 → scp/tar 到 GB10 `~/server/tts/GPT-SoVITS/`。
若只是日常部署、不重建,**不需要**此目录(镜像里已含代码)。

### GB10 构建坑(已写进 Dockerfile 注释,复述于此)

- 必须基于 `funasr-asr:arm64`,**不要装 torch/torchaudio**:只有该基础镜像的
  `torch 2.13.dev+cu130` 支持 GB10(sm_121)。
- 该 torchaudio 无 soundfile 后端、arm64 无 torchcodec → 两个 server 都用
  `sitecustomize`/内置垫片把 `torchaudio.load/save` 改到 soundfile。
- pip 走 Aliyun 镜像(`pypi.org`/Tsinghua 在该机 SSL 抖动);`--no-cache-dir`
  避免坏缓存;构建失败多为网络抖动,**重试即可**。
- GPT-SoVITS 需 `numpy<2`(已验证与 cu130 torch 兼容)、`gradio<5`(微调链路依赖)。

## bake-off 使用

CosyVoice 2(Windows PowerShell):
```powershell
curl.exe -s -o cosy.wav -F "tts_text=要说的话" -F "prompt_text=参考音频文字稿" `
  -F "prompt_wav=@C:\ref.wav;type=audio/wav" http://192.168.0.68:8095/tts/zero_shot
# 情感/语气:同上换 /tts/instruct,字段 instruct=用开心的语气说
```
`./compare.sh <ref.wav> "<文字稿>"`(在 GB10 跑)批量生成 零样本 + 多语气变体。

GPT-SoVITS v2Pro:
```bash
# 零样本(:8096 /tts,JSON)
curl -s -o gs.wav -H 'Content-Type: application/json' -d \
 '{"text":"要说的话","text_lang":"zh","ref_audio_path":"/io/ref.wav","prompt_text":"参考文字","prompt_lang":"zh"}' \
 http://192.168.0.68:8096/tts
# 每人微调(最高保真):把目标人录音放 ~/tts-io/,跑完自动热加载进 :8096
docker exec tts-gptsovits-1 python3 /io/gptsovits_finetune.py \
  --audio /io/<voice>.wav --exp <name> --epochs-s2 8 --epochs-s1 15 \
  --serve-url http://127.0.0.1:9880
```
正式微调建议目标人 5–30 分钟干净录音;ASR 底模已预置(首次不再等下载)。

## 文件清单

| 文件 | 作用 |
|---|---|
| `compose.tts.yaml` | 两服务编排(镜像/端口/卷),独立于生产 compose |
| `Dockerfile.cosyvoice` | CosyVoice2 镜像(基于 funasr-asr:arm64) |
| `cosy_server.py` | CosyVoice2 HTTP 服务(/health /tts/zero_shot /tts/instruct) |
| `Dockerfile.gptsovits` | GPT-SoVITS 镜像(COPY 本地 vendor 源码) |
| `gptsovits_sitecustomize.py` | torchaudio→soundfile 垫片(启动自动加载) |
| `gptsovits_finetune.py` | 无头微调:切片→FunASR→1abc→s2/s1 训练→热加载 |
| `compare.sh` | CosyVoice 批量 A/B 试听脚本 |
| `GPT-SoVITS/` | 第三方源码(**不入 git**;仅重建时需要,见上) |
