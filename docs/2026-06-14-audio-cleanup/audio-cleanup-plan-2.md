# Plan 2: 清洗服务 + HTTP 契约

## 前置依赖

Plan 1（Demucs GB10 验证）—— 其结论决定 Demucs stage 的 `device` 策略与 `max_duration_sec` 取值。

## 任务目标

在 `server/audio-cleanup/` 建一个独立的 aiohttp HTTP 服务（端口 `8097`），实现完整清洗管线
与 `POST /clean` / `GET /health`，含**同步限额与确定的错误体**。可被本机 curl 与 toolkit
（Plan 3）调用，输出干净音频文件。

## 执行范围

- **必须新增**：`server/audio-cleanup/{app.py,pipeline.py,Dockerfile,compose.cleanup.yaml,README.md}`。
- **允许新增**：宿主 `~/audio-cleanup/models/` 挂载约定（写进 README，不进镜像）。
- **禁止修改**：`server/asr/`、`server/orchestrator/`、`server/tts/`、`server/compose.yaml`、
  toolkit 仓任何文件（联动属 Plan 3）。
- **禁止**：把本服务并入生产 `server/compose.yaml`（必须独立 `compose.cleanup.yaml`）。

## 管线（顺序固定，每 stage 可单独开关）

```
输入(任意 ffmpeg 可解码) 
  → ① ffmpeg 解码 → PCM；探测时长，超 max_duration_sec 直接 422
  → ② Demucs htdemucs 取 vocals（separate=1 才跑；device 取 Plan 1 结论）
  → ③ DeepFilterNet 降噪+去混响（denoise=1）
  → ④ silero-vad 删停顿（pause=drop|duck|off）
  → ⑤ pyloudnorm 响度归一化（loudness!=off）
  → ⑥ 按请求 sr/format 重采样并编码输出
```

### 采样率契约（**关键约束，不可违反**）

`deep-filter` 当前**仅支持 48kHz**。因此：

| 规则 | 说明 |
|---|---|
| DF stage 内部**固定** 48kHz mono | 进 ③ 前若非 48k 必须先重采样到 48k mono；DF 始终在 48k 上工作 |
| `sr` 只在 ⑥ 生效 | 请求的 `sr`（16000/24000/48000）**只在最后一步**重采样输出，绝不在 DF 前按 `sr` 降采样 |
| Demucs / VAD 采样率 | Demucs 用其原生 44.1k→内部、VAD 用 16k；各 stage 间按需重采样，但 DF 必须见到 48k |

> 禁止「先按 sr=16000 重采样再进 DF」——会让 DF 行为不可控或直接失败。

### 激进度档位 `level`

| `level` | DeepFilterNet 衰减 | 适用 |
|---|---|---|
| `gentle` | 限制最大衰减，宁留底噪 | TTS 素材、ASR 预处理（默认 P1/P2） |
| `balanced` | 中等（默认 P0） | 给人听 |
| `aggressive` | 不限 | 底噪极重 |

> 选 DeepFilterNet 而非谱减/`noisereduce`：后者易把齿音/气声（s f p t）一起削掉变闷糊。

## 目标接口契约

### `POST /clean`

请求 `multipart/form-data`：

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `audio` | ✅ | — | 任意 ffmpeg 可解码音/视频；视频自动抽音轨 |
| `separate` | ❌ | `0` | `1` 开人声分离（Demucs） |
| `denoise` | ❌ | `1` | 降噪+去混响（DeepFilterNet，固定 48k） |
| `pause` | ❌ | `duck` | `drop`=删停顿段 / `duck`=压低保节奏 / `off`=不动 |
| `level` | ❌ | `balanced` | `gentle`/`balanced`/`aggressive` |
| `loudness` | ❌ | `-16` | 目标 LUFS；`off` 关 |
| `sr` | ❌ | `48000` | 输出采样率（仅 ⑥ 生效） |
| `format` | ❌ | `wav` | `wav`/`mp3`/`flac` |

成功 `200`：响应体为二进制音频（`Content-Type: audio/wav` 等），元数据放响应头：

```
X-Cleanup-Stages:   decode,separate,denoise,vad-duck,loudness
X-Cleanup-In-LUFS:  -28.3
X-Cleanup-Out-LUFS: -16.0
X-Cleanup-Duration: 4.2
```

### 同步限额与错误体（**必须固化为具名常量/env**）

| 常量 / env | v1 默认 | 含义 |
|---|---|---|
| `CLIENT_MAX_SIZE` | 512 MiB | aiohttp 上传上限 |
| `MAX_DURATION_SEC` | 600（GPU）/ 按 Plan 1 CPU 实测下调 | 音频时长上限；CPU 模式 Demucs ~1.5×音频时长，长视频易撞超时故下调 |
| `QUEUE_MAX` | 4 | 等待队列容量（不含正在处理的那 1 个）；满则立即 503 |
| `PROCESS_TIMEOUT_SEC` | 600 | 单请求处理墙钟上限 |
| `IDLE_UNLOAD_SEC` | 300 | 仅「常驻进程内模型」回退方案才用；v1 子进程模型下不使用（见「模型生命周期」） |

错误一律 `{"error":"<message>"}`（与 `/transcribe` 一致）：

| 状态 | 触发 | 处理建议 |
|---|---|---|
| `400` | 缺 `audio` / 非 multipart / 解析失败 | 检查字段名是 `audio` |
| `400` | `decode failed: ...` ffmpeg 失败 | 排查原文件 |
| `413` | 上传超 `CLIENT_MAX_SIZE` | 先转码/截取 |
| `422` | 音频时长超 `MAX_DURATION_SEC` | 切分后再传 |
| `503` | 等待队列超 `QUEUE_MAX`（busy） | 稍后重试，建议指数退避 |
| `504` | 处理超 `PROCESS_TIMEOUT_SEC` | `{"error":"processing exceeded 600s, split the input"}` |
| `500` | stage 内部异常 | 服务端日志有 traceback |

### `GET /health`

`{"model_loaded":bool,"stages_available":["separate","denoise","vad","loudness"],"gpu":bool}`。

## 行为规则

| 请求 | 处理路径 | 输出 |
|---|---|---|
| `separate=1 denoise=1 pause=off sr=16000`（douyin/ASR） | 解码→Demucs vocals→DF(48k)→归一化→重采样 16k | 干净人声 wav，去 BGM |
| `denoise=1 pause=duck`（默认 P0 给人听） | 解码→DF(48k)→VAD 压低停顿→归一化→48k | 干净录音，保留节奏 |
| `separate=1 level=gentle`（TTS 素材） | 解码→Demucs→DF 温和→归一化 | 高保真人声，音色不被美化 |
| 音频时长 > `MAX_DURATION_SEC` | 解码后探测时长即拒 | `422 {"error":...}` |
| worker 忙且队列满 | 不入队 | `503 {"error":"busy"}` |

## 部署

`server/audio-cleanup/compose.cleanup.yaml`，**独立 compose**，端口 `8097`：

- **restart 策略：`restart: unless-stopped`**（拍板）。理由：被 douyin **自动管线**消费，需 GB10
  重启后自愈，与 `server/tts/compose.tts.yaml` 一致。

### 模型生命周期（**消除「常驻 GPU 仍说空闲低」的矛盾**）

「懒加载」单独并不能让空闲显存低——首次请求载入后若常驻 GPU，空闲时依然占着。

**本服务的可终止子进程模型（见 §Agent 执行步骤）天然解决了这点**：pipeline 每单跑在独立子进程里，
**模型在子进程内加载、随子进程退出而释放**——每单结束 GPU 即归还，空闲时无任何常驻占用，也无需
单独的 idle-TTL 卸载逻辑（故 `IDLE_UNLOAD_SEC` 仅在万一改回「常驻进程内模型」时才需要，v1 子进程
模型下不使用）。

剩下要由 Plan 1 拍板的只是 **device**：

| 方案 | 适用 | 实现动作 |
|---|---|---|
| A. Demucs 固定 CPU | GPU 不兼容或抢显存严重 | 子进程内 `device="cpu"`；DF 本就 CPU 实时；GPU 全程零占用 |
| **B. Demucs GPU**（✅ **已拍板**） | Plan 1 实测 GPU 可用且收益大 | 子进程内 `device="cuda"`；子进程退出即释放显存，无需手动卸载 |

> **2026-06-15 Plan 1 spike 定论：取方案 B（`CLEAN_DEMUCS_DEVICE=cuda`）**——cuda gpu_peak
> 仅 0.91GB、净算约 cpu 的 4×。Dockerfile/compose 默认已设 cuda，降级排查可 env 切 cpu。
> 无论 A/B，**禁止把模型常驻在主进程/事件循环里**——必须随子进程生灭，保证空闲零显存。
- **端口暴露**：默认仅 `127.0.0.1:8097`（同机 douyin 直连足够）；zero-desktop 经 toolkit-server
  代理（Plan 3），**不**默认绑 `0.0.0.0`/LAN。
- **模型权重**：放宿主 `~/audio-cleanup/models/`，容器 `:ro` 挂载，不进镜像；走
  `hf-mirror.com`/ModelScope 下载。
- **GB10 构建坑**（沿用 `server/tts` README）：基于 `funasr-asr:arm64`、不重装 torch、torchaudio
  monkeypatch→soundfile、pip 走 Aliyun + `--no-cache-dir`。

## Agent 执行步骤

1. 新增 `server/audio-cleanup/Dockerfile`：`FROM funasr-asr:arm64`；pip 走 Aliyun + `--no-cache-dir`
   安装 `demucs`、`deepfilternet`、`pyloudnorm`、`onnxruntime`（silero）、`aiohttp`；**不重装**
   `torch`/`torchaudio`；启动时 monkeypatch `torchaudio.load/save → soundfile`。
   **安装后加构建期断言**校验 torch 未被这些依赖偷偷升级：
   `RUN python -c "import torch; v=torch.__version__; assert v.startswith('2.13') and 'cu130' in v, v"`；
   断言失败即构建失败。README 的冒烟段也写一条同样的运行期校验命令，并**附带打印
   `torchaudio.__version__`**（仅记录、不硬断言——dev 版版本串不好匹配，软校验即可）。
2. 新增 `server/audio-cleanup/pipeline.py`：实现 `decode/separate/denoise/vad/loudness/encode` 六个
   stage 函数，每个独立可测。**DF stage 内部固定 48k mono**；`sr` 只传给 `encode`。silero VAD 复用
   `src-tauri/assets/silero_vad.onnx` 同款模型（拷一份到挂载目录）。
3. 在 `pipeline.py` 实现**模型生命周期**：按 Plan 1 结论选方案 A（Demucs 固定 `device="cpu"`，无卸载）
   或 B（懒加载 + 后台任务空闲 `IDLE_UNLOAD_SEC` 后 `del` + `torch.cuda.empty_cache()`）。禁止载入后
   无限常驻 GPU。
4. 新增 `server/audio-cleanup/app.py`（aiohttp）：
   - **并发控制必须实现**：用**单 worker** 模式——全局 `asyncio.Semaphore(1)` 包住「解码→…→编码」
     完整 pipeline（aiohttp 默认会并发处理请求，不加锁会同时跑多个 pipeline 抢资源）；另设一个
     计数器/`asyncio.Queue` 记录**等待中**请求数，超过 `QUEUE_MAX` 时**立即返回 503**（不入队、不等待）。
   - **可终止执行（防后台泄漏）**：pipeline 重活必须跑在**可被杀死的子进程**里
     （`multiprocessing.Process` 或 `asyncio.create_subprocess_*`），**不可**用裸 `run_in_executor`
     线程——线程超时后无法真正 kill，会泄漏后台任务、占着资源还释放了锁去接下一单。超时到
     `PROCESS_TIMEOUT_SEC` 时：先 `terminate()`→必要时 `kill()` 子进程，**等待其真实回收
     （join/wait）后**再释放并发锁、返回 504。即「锁的持有时间 ≥ 子进程实际存活时间」，杜绝
     一个 504 后旧任务仍在跑、新任务又进来的叠加。
   - 解码后探测时长，超 `MAX_DURATION_SEC` 返回 422；`client_max_size=CLIENT_MAX_SIZE` 超限 413。
   - 注册 `POST /clean`、`GET /health`；所有错误体统一 `{"error": "..."}`。
5. 把 `CLIENT_MAX_SIZE/MAX_DURATION_SEC/QUEUE_MAX/PROCESS_TIMEOUT_SEC/IDLE_UNLOAD_SEC` 定义为
   **模块级具名常量**（可被同名 env 覆盖），禁止散落 magic number。
6. 新增 `server/audio-cleanup/compose.cleanup.yaml`：端口仅 `127.0.0.1:8097:8097`、`restart:
   unless-stopped`、`:ro` 挂载 `~/audio-cleanup/models`。
7. 新增 `server/audio-cleanup/README.md`：构建/部署/挂载/构建坑/冒烟 runbook。

## 禁止事项

- 不要把 `sr` 应用在 DF stage 之前（违反 48k 契约）。
- 不要省略并发控制——aiohttp 默认并发，必须用 Semaphore(1) + bounded queue 包住完整 pipeline。
- 不要让模型载入后无限常驻 GPU（必须走方案 A 或 B）。
- 不要用裸 `run_in_executor` 线程跑 pipeline——超时杀不掉，必须用可终止子进程。
- 不要在子进程未真实回收前释放并发锁（防 504 后任务叠加）。
- 不要并入生产 `server/compose.yaml`。
- 不要把超时/上限/队列写成散落 magic number——必须是具名常量或 env。
- 不要默认绑 `0.0.0.0` 或暴露到 LAN。
- 不要在本 Plan 改 toolkit 仓任何文件（属 Plan 3）。
- 不要静默吞 stage 异常——失败按上表返回对应状态 + `{"error":...}`。

## 测试 / 验证要求

- 单测 `pipeline.py` 每个 stage：DF 输入非 48k 时确认内部重采样到 48k；`sr` 仅末端生效。
- 单测限额分支：构造超时长输入→断言 422；构造 `QUEUE_MAX+1` 并发→断言队列满那个返回 503；
  超时→断言 504 错误体文案。
- 并发控制单测：断言同一时刻最多 1 个 pipeline 在跑（Semaphore(1) 生效），等待计数正确。
- 超时回收单测：构造超 `PROCESS_TIMEOUT_SEC` 的任务→断言子进程被 kill 且**真实回收后**才释放锁、
  返回 504；随后下一单能正常拿到锁（无叠加）。
- 冒烟（GB10）：
  ```bash
  curl -sS -F audio=@noisy.wav -F denoise=1 -F pause=duck http://127.0.0.1:8097/clean -o out.wav
  curl -sS -F audio=@bgm_video.mp4 -F separate=1 -F pause=off -F sr=16000 http://127.0.0.1:8097/clean -o vocals.wav
  curl -s http://127.0.0.1:8097/health   # model_loaded / stages_available / gpu
  ```
  断言 `out.wav`/`vocals.wav` `ffprobe` 时长 > 0；health 返回 `stages_available` 含四项。

## 完成条件

- [ ] `server/audio-cleanup/{app.py,pipeline.py,Dockerfile,compose.cleanup.yaml,README.md}` 已建
- [ ] Dockerfile 含 torch 版本断言（`2.13`+`cu130`）；README 有同款运行期校验命令
- [ ] 管线五 stage 实现，DF 固定 48k、`sr` 仅末端生效（单测覆盖）
- [ ] `/clean` 支持全部 multipart 字段；限额 413/422/503/504 + `{"error":...}` 错误体（单测覆盖）
- [ ] 并发控制：Semaphore(1) 包住完整 pipeline + bounded queue（满即 503），单测验证
- [ ] pipeline 跑在可终止子进程；504 超时杀进程且真实回收后才释放锁（超时回收单测通过）
- [ ] 模型随子进程生灭、不常驻主进程（device 按 Plan 1 取 A/B），空闲 GPU 零占用
- [ ] `/health` 返回三字段
- [ ] `compose.cleanup.yaml` 用 `restart: unless-stopped`、仅绑 `127.0.0.1:8097`
- [ ] GB10 镜像构建成功，两条冒烟 curl 通过
- [ ] `README.md` 含部署/挂载/构建坑/冒烟 runbook
