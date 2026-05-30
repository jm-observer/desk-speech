# server/tts/legacy/ — GPT-SoVITS bake-off 残留

2026-05-28 的 TTS bake-off 选定 **CosyVoice2** 作为唯一线上引擎,
GPT-SoVITS v2Pro 路线**已放弃**(跨语言英文崩、短句易截断、无情感控制)。

本目录是当时的微调流水线 + A/B 试听脚本,保留作历史回溯,**不再编排部署**:

| 文件 | 作用 |
|---|---|
| `Dockerfile.gptsovits` | GPT-SoVITS v2Pro 镜像(基于 funasr-asr:arm64) |
| `gptsovits_finetune.py` | 切片 → ASR → 特征 → 训练的微调流水线 |
| `gptsovits_sitecustomize.py` | torchaudio → soundfile 后端的 monkeypatch |
| `compare.sh` | bake-off 时期的 cosy/gs 批量 A/B 试听脚本 |

**重新启用**(仅当需要再跑一轮对比):

1. 在能访问 github 的机器 clone `RVC-Boss/GPT-SoVITS` 到 `server/tts/GPT-SoVITS/`
   (该目录在 `.gitignore`,不入 git)
2. 在 `server/tts/compose.tts.yaml` 加回 `gptsovits` service(参见 git
   commit `1846ca4` / `6fe70fd` 的历史定义)
3. `docker compose -f compose.tts.yaml up -d --build gptsovits`

线上服务的运维/调用见上一级 `../README.md` 与 `../API.md`。
