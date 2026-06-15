# Plan 1 spike — Demucs GB10 风险验证

目的：在投入服务骨架前，**确定性地**回答「htdemucs 在 GB10 能否跑、跑 GPU 还是 CPU、单位
时长多慢」，据此定 Plan 2 的 `device` 策略与 `MAX_DURATION_SEC`。详见
[`docs/2026-06-14-audio-cleanup/audio-cleanup-plan-1.md`](../../../docs/2026-06-14-audio-cleanup/audio-cleanup-plan-1.md)。

## 跑法（在 GB10 上）

```bash
# 0) 准备一段测试 wav（~30s，含人声+背景音乐最理想），放到本目录命名 test.wav
#    本地 scp：  scp server/audio-cleanup/spike/* fengqi@192.168.0.68:~/spike-demucs/
#    或在 GB10 上用 ffmpeg 从任意带乐视频抽 30s：
#      ffmpeg -i some.mp4 -t 30 -ac 2 -ar 44100 test.wav

# 1) 构建 spike 镜像（torch 版本断言在构建期就会拦截依赖漂移）
cd ~/spike-demucs
docker build -f Dockerfile.spike -t demucs-spike:gb10 .

# 2) 跑验证（挂载 test.wav；htdemucs 权重首次会下载，GitHub 不稳则需先 hf-mirror/手动放置）
docker run --rm --runtime=nvidia -e NVIDIA_VISIBLE_DEVICES=all \
  -v "$PWD/test.wav:/spike/test.wav:ro" \
  -v "$HOME/.cache/torch:/root/.cache/torch" \
  demucs-spike:gb10
```

> htdemucs 权重默认从 GitHub release 拉，GB10 直连不稳。若下载超时：在能联网的机器下好
> `955717e8-8726e21a.th` 等权重，放到挂载的 `~/.cache/torch/hub/checkpoints/`。

## 实测结论（2026-06-15，GB10 实跑）

版本串（`run_spike.py` 启动打印）：

- torch = `2.13.0.dev20260517+cu130` ✅（Dockerfile 构建期断言通过，demucs 未升级 torch）
- torchaudio = `2.11.0.dev20260518+cu130`
- `cuda_available = True`

结论表（30s 立体声 test.wav，htdemucs 4 轨分离，vocals_idx=3）：

| device | 是否成功 | 耗时 | 单位时长耗时 | 显存峰值 | 备注 |
|--------|---------|------|------------|---------|------|
| **cuda** | ✅ | 9.9s（含一次性权重下载 ~7.2s） | **≈0.1 s/音频s**（净算 ~2.7s/30s） | **0.91 GB** | htdemucs 模型小，显存占用极低 |
| **cpu**  | ✅ | 11.3s（权重已缓存） | ≈0.38 s/音频s | — | 约 cuda 净算的 4× 慢 |

## 拍板：方案 B（Demucs on GPU，`CLEAN_DEMUCS_DEVICE=cuda`）

理由：cuda 与 cpu 均可跑；**cuda 显存峰值仅 0.91 GB**（对 asr/tts/vLLM 几乎无挤占，且子进程
退出即释放），净算速度约 cpu 的 4×。故 Plan 2 默认走 **cuda**。

- `MAX_DURATION_SEC=600` 保持：GPU 上 Demucs ~0.1 s/音频s，10 分钟音频 Demucs 段 ~60s，
  远低于 `PROCESS_TIMEOUT_SEC=600`。
- Plan 2 GB10 冒烟（2026-06-15）已验全链路：separate+denoise 30s 音频 GPU 上 ~3.0s
  （~0.1 s/音频s，含 Demucs+DF）。**长音频（接近 10min）DF 时长未单独实测**——按此速率 10min
  约 60-120s，远低于 `PROCESS_TIMEOUT_SEC=600`；若日后长输入频繁 504，再按实测下调 `MAX_DURATION_SEC`。
- 权重 `955717e8-8726e21a.th`（80 MB）已缓存到 `~/spike-demucs/torch-cache/`（GitHub 直连此次成功，
  ~11.6 MB/s）。Plan 2 容器挂 `~/audio-cleanup/torch-cache` 复用，避免重下。
