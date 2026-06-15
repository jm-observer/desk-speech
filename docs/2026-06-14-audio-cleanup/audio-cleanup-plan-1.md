# Plan 1: Demucs GB10 风险验证（spike）

## 前置依赖

无。本 Plan 是整个任务的**解风险前置**，必须最先执行。

## 任务目标

在 GB10（arm64 + CUDA13，`funasr-asr:arm64` 基础镜像，`torch 2.13.dev+cu130`）上**确定性地
证明** Demucs `htdemucs` 能否完成人声分离，并产出一个**可执行结论**：

- ✅ GPU 可跑 → 记录单位时长耗时、显存峰值；
- ⚠️ GPU 算子不兼容但 CPU 可跑 → 记录 CPU 单位时长耗时（用于 Plan 2 定 `max_duration_sec`）；
- ❌ 均不可跑 → 给出备选（MDX-Net / spleeter）的验证结论。

本 Plan **不写服务**，只产出一份验证记录，供 Plan 2 取参数。

## 执行范围

- **允许新增**：`server/audio-cleanup/spike/`（仅验证脚本与 Dockerfile，临时；Plan 2 可复用或删除）。
- **禁止修改**：`server/asr/`、`server/orchestrator/`、`server/tts/`、`server/compose.yaml`、
  任何生产容器编排。
- **禁止**：在 GB10 生产栈容器内安装 demucs/torch（必须在隔离的 spike 镜像里验证）。

## Agent 执行步骤

1. 新增 `server/audio-cleanup/spike/Dockerfile.spike`：`FROM funasr-asr:arm64`；pip 走
   Aliyun 镜像 + `--no-cache-dir` 安装 `demucs`；**不得**重装 `torch`/`torchaudio`
   （复用基础镜像的 `torch 2.13.dev+cu130`）。**安装后必须加一行构建期断言**校验 torch 未被
   依赖偷偷升级——`RUN python -c "import torch; v=torch.__version__; assert v.startswith('2.13') and 'cu130' in v, v"`；
   断言失败即构建失败（暴露 demucs 把 torch 拉成 PyPI 正式版的情况）。
2. 新增 `server/audio-cleanup/spike/run_spike.py`：**启动即打印 `torch.__version__` 与
   `torchaudio.__version__`**（torch 已在 Dockerfile 硬断言；torchaudio 仅打印记录、不硬断言——
   某些 dev 版版本串不好匹配，软校验即可）；随后加载 `htdemucs`，对一段测试 wav 跑分离，
   分别尝试 `device="cuda"` 与 `device="cpu"`；每种 device 必须打印：成功/失败、异常类型全文、
   wall-clock 耗时、`torch.cuda.max_memory_allocated()`（GPU 路径）。
3. 新增 `server/audio-cleanup/spike/README.md`：记录构建命令、scp 路径、运行命令、**实测结论
   表**（device × 成功 × 耗时 × 显存），并记下 `run_spike.py` 打印的 torch / torchaudio 版本串
   （供日后排查依赖漂移）。
4. 在 GB10 上构建 spike 镜像并运行（GitHub 不稳，权重走 `hf-mirror.com`/ModelScope；见
   CLAUDE.md「GB10 network gotchas」）。
5. 把实测结论表回填进 spike README，并据此在总览文档「风险与待定项」的 Demucs 行更新结论。

## 目标产物 / 接口契约

无对外接口。产物是 `server/audio-cleanup/spike/README.md` 中的结论表：

```
| device | 是否成功 | 单位时长耗时(s/音频s) | 显存峰值(GB) | 异常摘要 |
|--------|---------|---------------------|-------------|---------|
| cuda   | ?       | ?                   | ?           | ?       |
| cpu    | ?       | ?                   | —           | ?       |
```

## 行为规则

| 验证输入 | 期望产出 |
|---|---|
| `htdemucs` + `device="cuda"` 加载并分离 30s 测试 wav | 记录成功/失败 + 耗时 + 显存；失败则记完整异常类型 |
| `htdemucs` + `device="cpu"` 分离同一 wav | 记录成功/失败 + 耗时；用于 Plan 2 计算 CPU 模式 `max_duration_sec` |
| GPU、CPU 均失败 | 追加验证 `spleeter` 或 `MDX-Net`，记录其结论 |

## 禁止事项

- 不要在本 Plan 实现 HTTP 服务、`/clean` 端点或任何管线 stage（DF/VAD/loudness）。
- 不要修改任何生产容器或 `server/compose.yaml`。
- 不要重装基础镜像里的 `torch`/`torchaudio`。
- 不要把 spike 镜像加入任何生产 compose。
- 不要为了「让它过」而静默吞掉 CUDA 异常——异常必须完整记录，这正是本 Plan 的价值。

## 测试 / 验证要求

- 在 GB10 执行 `python run_spike.py`，**stdout 必须包含** cuda 与 cpu 两段结果（成功或异常全文）。
- spike README 的结论表四列填满，无 `?` 残留。
- 冒烟：分离输出的 `vocals.wav` 可被 `ffprobe` 正常读出时长 > 0。

## 完成条件

- [ ] `server/audio-cleanup/spike/{Dockerfile.spike,run_spike.py,README.md}` 已创建
- [ ] Dockerfile.spike 含 torch 版本断言（`2.13`+`cu130`），构建通过即证明 demucs 未升级 torch
- [ ] GB10 上 spike 镜像构建成功
- [ ] `run_spike.py` 在 GB10 跑出 cuda + cpu 两条结论（成功或带完整异常）
- [ ] spike README 结论表填满
- [ ] 总览文档「风险与待定项」Demucs 行已据实测更新（GPU/CPU/备选 三选一定论）
- [ ] 已向用户报告结论，确认 Plan 2 采用的 device 策略
