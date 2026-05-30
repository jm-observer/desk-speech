# asr-server 多格式输入 + VAD 切段 + 同机路径端点

> 时间：2026-05-30
> 范围：仅 `server/asr-server/`，与 orchestrator/asr/tts 互不影响
> 目的：让外部调用方（zero 的 douyin skill、未来其它视频内容理解链路）
> 直接把 mp4 / 长视频喂进来拿文本，不必在自己一侧装 ffmpeg、切段、
> 上传整段 multipart。
>
> 关联背景：zero 项目计划把抖音博主作品批量转文本汇总成知识包，
> 视频是 mp4、时长 1-3 分钟，跨机调用一台 multipart 上传 1-3 MB
> 已经偏胖；最终调用方式预期是同机文件路径直传。

---

## 1. 项目现状

`server/asr-server/src/main.rs` 单文件最小实现：

- 接口：`GET /healthz` / `GET /v1/models` / `POST /v1/audio/transcriptions`
- `transcribe`（multipart 入口）从 `file` 字段取 bytes → `decode_wav_16k_mono`
  （WAV 快路径解码）→ sherpa-onnx 推理 → 返 `{"text":"..."}`
- 模型由启动参数 `--model {sense-voice|whisper-turbo}` 固定
- 已知限制（`server/asr-server/README.md` §"已知限制"）：
  1. **只接受 16 kHz 单声道 WAV**，其它格式/采样率需调用方自己 ffmpeg
  2. **Whisper 单段 ≤30 s**，更长音频需调用方切段
  3. **模型/语言不能按请求切换**（仍按启动参数）
  4. **无鉴权**
  5. **`Mutex<OfflineRecognizer>` 串行解码**

本 plan 解决 1、2，并新增**同机路径直读端点**。3、4、5 不在本次范围。

---

## 2. 整体目标

| 现状痛点 | 改后行为 |
|---|---|
| 调用方必须自己 `ffmpeg -ar 16000 -ac 1 in.mp4 out.wav` | 服务端检测格式，内部 ffmpeg 转码到 16 kHz 单声道 PCM |
| Whisper 30 s 限制把 1 分钟以上视频卡死 | 可选 VAD 切段（移植 `src-tauri/src/commands/recording.rs` 的 silero_vad 循环），按段推理，返回 `segments[]` |
| 同机调用还要走 multipart 上传 mp4 字节流 | 新增 `POST /v1/audio/transcriptions/from-source`，body 传文件本地路径或 HTTP URL |
| 现有 `{ "text": "..." }` 调用方期待不被破坏 | 协议向后兼容：未开启 vad 时仍只回 `text` 字段；现有 16kHz mono WAV 调用零修改可继续工作 |

---

## 3. Plan 拆分

| Plan | 标题 | 前置 | 状态 |
|---|---|---|---|
| **A** | 多格式输入（ffmpeg 内嵌） | 无 | 待启动 |
| **B** | VAD 切段 + 多段返回 | A 完成（A 把 PCM 解码统一了） | 待启动 |
| **C** | `from-source` 端点（路径 / URL） | A 完成 | 待启动 |

A 是地基（统一 "任意输入 → 16k mono PCM" 这一步），B 和 C 都依赖它。
B 和 C 之间没有依赖，可并行或单独裁掉。

---

## Plan A：多格式输入

### 前置依赖
无。

### 任务目标
调用方上传任意 ffmpeg 支持的音视频文件（mp4/mp3/m4a/webm/ogg/flac/...），
服务端内部转码到 16 kHz 单声道 f32 PCM 后送 sherpa-onnx，返回与现状
一致的 `{"text":"..."}`。**16kHz mono WAV 仍走快速路径不经 ffmpeg**。

### 执行范围
- **必须修改**：`server/asr-server/src/main.rs`、`server/asr-server/Cargo.toml`、
  `server/asr-server/Dockerfile`、`server/asr-server/README.md`
- **允许修改**：`server/compose.yaml`（如要给 asr-server profile 加资源限制）
- **禁止触碰**：`server/asr/`（FunASR 容器，独立线）、`server/orchestrator/`、
  `src-tauri/`、`src/`

### 实现要点

1. **`Dockerfile`**：base image 加 `ffmpeg`（`apt-get install -y ffmpeg`，
   arm64 ubuntu 直接有）。注意镜像体积，ffmpeg 静态版本约 60 MB。
   ⚠️ GB10 上 GitHub 不稳，但 `ffmpeg` 来自 ubuntu apt（清华源代理已配），
   不走 GitHub。

2. **`main.rs::transcribe`**：判断输入：
   - **快路径**：bytes 头 4 字节 == `RIFF`，**且** WAV `fmt ` chunk 满足
     全部 4 个条件：`channels == 1`、`sample_rate == 16000`、`bits_per_sample ∈ {16, 32}`、
     `audio_format ∈ {1=PCM, 3=IEEE Float}` → 直接走现有 `decode_wav_16k_mono`
   - **慢路径**：其它情况（多声道 / 非 16k / ADPCM 等异常子格式 / 非 WAV）
     → 走 ffmpeg pipe(stdin) → 拿 16k mono PCM(stdout)
   - 判定快/慢只读 WAV 头 44 字节，不读完整 payload，廉价

3. **ffmpeg 调用**：用 `tokio::process::Command`：

   ```rust
   // 出于安全：永远从 stdin 喂 bytes，从 stdout 拿 wav，不落盘
   // -f s16le 输出 PCM；-ar 16000 -ac 1 单声道 16k
   // -hide_banner -loglevel error 静音
   ffmpeg -hide_banner -loglevel error -i pipe:0 \
          -f s16le -ar 16000 -ac 1 -acodec pcm_s16le pipe:1
   ```

   stdin 写入原始 bytes，stdout 读到 i16 PCM，转 f32 喂 sherpa-onnx。
   全程不落盘，避免临时文件清理 + 路径注入风险。

4. **超时**：ffmpeg subprocess 加 `tokio::time::timeout`，
   默认 60 s（比抖音视频长度+一倍余量），超时 kill 子进程 +
   返回 `error.type = "decode_timeout"`。

5. **错误分类**：
   - ffmpeg exit ≠ 0 → `400 invalid_request "ffmpeg decode failed: <stderr 前 200 字符>"`
   - ffmpeg timeout → `400 invalid_request "decode timeout"`
   - 输出长度异常（< 0.1 s 音频）→ `400 invalid_request "decoded audio too short"`

### 目标接口契约

`POST /v1/audio/transcriptions`：
- `file` 字段语义放宽：从 "必须 16 kHz 单声道 WAV" → "任意 ffmpeg 可解的音视频文件"
- 响应不变：`{"text":"..."}` 或 `{"error":{"message":"...","type":"..."}}`
- 现有 16 kHz mono WAV 调用方**零修改**继续工作

### 行为规则

| 输入 | 行为 |
|---|---|
| 16 kHz mono WAV（PCM16/32/Float） | 快路径，直接 hound 解码 |
| 其它 WAV（采样率非 16k / 多声道 / IEEE Float 之外编码） | 走 ffmpeg 重采样 |
| mp4 / mp3 / m4a / webm / ogg / flac / mka | 走 ffmpeg 转码 |
| 既非 RIFF 又 ffmpeg 解失败 | `400 invalid_request "ffmpeg decode failed: ..."` |
| 解码出的音频 < 0.1 s | `400 invalid_request "decoded audio too short"` |
| ffmpeg 超过 60 s 仍未完成 | kill + `400 invalid_request "decode timeout"` |

### 禁止事项

- 不把临时文件落到磁盘（stdin/stdout pipe）
- 不引入 symphonia / rubato 等纯 Rust 解码栈（理由：ffmpeg 覆盖更全、
  解码精度业界标准；纯 Rust 路线日后真有必要再说，避免一次性双轨）
- 不改动 sherpa-onnx 推理路径（`spawn_blocking` 块）

### 测试要求

- `tests/transcribe_multifmt.rs` 集成测试：
  - `test_wav_16k_mono_fast_path`：发 1 s 的 16k mono sine wave → 200 OK + 有 text
  - `test_wav_44k_stereo_via_ffmpeg`：发 44.1 kHz 立体声 WAV → 200 OK + 有 text
  - `test_mp3_via_ffmpeg`：发 0.5 s sine 编码 mp3 → 200 OK
  - `test_mp4_audio_track`：发带 AAC 音轨的 mp4 → 200 OK
  - `test_garbage_bytes`：发 32 字节 `b"not an audio file at all"` → 400 + type=invalid_request
  - `test_no_audio_stream`：发只有视频轨的 mp4 → 400
- 验证命令：`cd server/asr-server && cargo test -- --test-threads=1`
  （sherpa-onnx 模型加载耗内存，串行测试）

### 完成条件

- [ ] Dockerfile 包含 ffmpeg，镜像能 build
- [ ] 上述 6 个测试全绿
- [ ] `curl -F file=@some.mp4 http://192.168.0.68:8091/v1/audio/transcriptions` 拿到文本
- [ ] 现有 16 kHz mono WAV 调用方实测无回归
- [ ] `server/asr-server/README.md` "已知限制" 删掉第 1 条

---

## Plan B：VAD 切段 + 多段返回

### 前置依赖
Plan A 完成（PCM 解码已统一）。

### 任务目标
对长音频（典型场景：1-3 分钟抖音视频），服务端用 silero_vad 切成语音段，
每段单独解码，返回 `segments[]` 含每段 `start/end/text`。突破 Whisper ≤30 s
限制；SenseVoice 虽无硬上限但长音频精度也下降，VAD 切段对它同样有益。

### 执行范围
- **必须修改**：`server/asr-server/src/main.rs`、`server/asr-server/Cargo.toml`
- **新增**：`server/asr-server/src/vad.rs`（独立模块）
- **必须新增到镜像**：`silero_vad.onnx` 模型文件（与 `src-tauri/assets/` 同款）

### 实现要点

1. **VAD 移植**：参考 `src-tauri/src/commands/recording.rs` 的 silero_vad
   循环逻辑，抽取到独立 `vad.rs`。**不**直接共享 src-tauri 的代码
   （workspace 两 crate 故意独立），而是复制 + 标注来源。

2. **触发开关**：multipart 加可选字段 `vad`：
   - `vad` 缺省 / `false` / `"0"`：行为同 Plan A，返回单 text
   - `vad=true` / `"1"`：触发 VAD 切段，返回 `segments[]`

3. **每段独立推理**：每段 PCM 送 sherpa-onnx 一次。串行（受
   `Mutex<OfflineRecognizer>` 限制）；后续如需优化做 recognizer pool。

4. **二次切段**：如某段 > 30 s（Whisper 模式硬限）→ 在该段内按
   **25 s 窗 + 2 s 重叠**（即步长 23 s）硬切，避免 Whisper 截断。
   合并文本时**重叠区取后一窗的识别结果**（前窗在边界处更易吞字）。
   合并后的 segment 时间戳取**整段 VAD 的 `start`/`end`**，不细化子窗时间。
   SenseVoice 模式不需要这层。

5. **响应模型**：

   ```json
   {
     "text": "全部段拼接的文本",
     "segments": [
       {"start": 0.0, "end": 4.2, "text": "..."},
       {"start": 4.8, "end": 9.5, "text": "..."}
     ]
   }
   ```

   `text` 字段保留，等价 `segments.map(s => s.text).join(" ")`，方便不关心
   时间轴的调用方继续按现状取值。

### 目标接口契约

`POST /v1/audio/transcriptions`：
- 新增可选字段 `vad`（boolean / "true"/"false"/"0"/"1"）
- 响应：`vad=true` 时增加 `segments[]`，`text` 字段始终存在

### 行为规则

| 输入 | 行为 |
|---|---|
| 任意输入 + 不传 `vad` | 与 Plan A 行为完全一致 |
| 任意输入 + `vad=true` | VAD 切段 → 逐段推理 → 返 `text` + `segments[]` |
| `vad=true` 但音频 < 1 s | 跳过 VAD，整段直推，`segments` 退化为单元素 |
| `vad=true` + whisper 模式 + 某段 > 30 s | 段内 25 s 滑窗硬切，段内多次推理后再合并到一个 segment |

### 禁止事项

- 不在 VAD 模式下做"段间润色"或 LLM 后处理（这是 orchestrator 的事）
- 不返回 `segments[].score`、`segments[].lang` 等 sherpa-onnx 不稳定字段
- 不并发推理（受 `Mutex<OfflineRecognizer>` 限制，先串行；并发优化是另一个 plan）

### 测试要求

- `tests/transcribe_vad.rs`：
  - `test_short_audio_passthrough`：0.5 s 音频 + `vad=true` → 退化为单段
  - `test_two_speech_segments_with_silence`：构造 "1s 语音 + 2s 静音 + 1s 语音"
    → 2 个 segments，时间戳大致对得上
  - `test_long_audio_whisper_subsegment`：构造 45 s 持续语音 + whisper 模式 →
    内部至少 2 次推理，返回 1 个 segment（合并后）
- 验证命令：`cd server/asr-server && cargo test`

### 完成条件

- [ ] `silero_vad.onnx`（~2 MB）通过 Dockerfile `COPY` 进镜像
      （**不**走挂载——避免再造一个外部契约；挂载留给 sherpa 那种 >100 MB 的生产模型）
- [ ] 上述 3 个测试全绿
- [ ] curl 实测 1 分钟视频 + `vad=true` 拿到 ≥1 个段
- [ ] `README.md` "已知限制" 删掉第 2 条

---

## Plan C：`from-source` 端点

### 前置依赖
Plan A 完成（任意输入 → PCM 已统一）。Plan B 可选（B 完成后 from-source 自动
支持 vad 参数）。

### 任务目标
同机调用方（zero 在 GB10 本机下载了抖音 mp4 到 `~/.config/zero/downloads/`）
可以直接传**文件路径**而不是 multipart 上传整段字节流，省一次拷贝。也支持
HTTP URL（服务端下载到临时文件再走 Plan A 路径）。

### 执行范围
- **必须修改**：`server/asr-server/src/main.rs`、`server/asr-server/Cargo.toml`（加 reqwest）
- **必须新增**：`--source-allowlist` 启动参数

### 实现要点

1. **新端点**：`POST /v1/audio/transcriptions/from-source`
   ```json
   {"source": "file:///abs/path/to/aweme.mp4", "vad": true}
   ```
   或
   ```json
   {"source": "https://example.com/audio.mp3", "vad": false}
   ```

2. **`file://` 解析规则**（先合法、再白名单，分两层判）：
   - **合法**：必须形如 `file:///<abs-posix-path>`（三斜杠 + 绝对路径）；
     **拒绝 URL 编码字符**——出现 `%` 一律 400 `invalid_request`
     "encoded file:// path not supported"（避免 `%2e%2e` 之类绕过 canonical
     检查、以及中文路径的多重解码歧义）
   - 拒绝 Windows 风格 `file:///C:/...`（本服务只跑在 GB10 linux）
   - **白名单**：启动参数 `--source-allowlist /home/fengqi/.config/zero/downloads/,/tmp/asr-input/`
     （逗号分隔，多前缀）。解析出的路径先 `canonicalize` 再做 prefix
     比对，**白名单本身也要 canonical 化**，避免 symlink 和 `..` 逃逸
   - 未传 `--source-allowlist` → `from-source` 端点禁用（启动日志 warn）
   - 白名单不匹配 → `403 forbidden_source "path not in --source-allowlist"`

3. **HTTP URL 下载**（流式，绝不全量读进内存等量级判断）：
   - 只支持 `http://` `https://`
   - 落到 `/tmp/asr-input/<uuid>.bin`（不靠 URL 扩展名猜，ffmpeg 自识别）
   - **流式**：`reqwest::Response::bytes_stream()` 边读边写边累计字节数；
     超过 `--max-source-bytes`（默认 100 MB）立刻 drop stream + 删临时文件
     + 返回 `400 invalid_request "fetch too large"`
   - 整次下载受 `--source-fetch-timeout`（默认 30 s）的 `tokio::time::timeout`
     约束；超时同样删临时文件 + 400
   - 处理完无论成功失败一律删临时文件（`Drop` guard 兜底）

4. **复用核心**：内部把 source 转成 bytes 后，走 Plan A 的相同 ffmpeg
   pipeline；Plan B 的 vad 参数继续生效。

5. **绑定 127.0.0.1**（安全相关，from-source 上线一并改）：
   - 当前 asr-server 默认 `--bind 0.0.0.0:8091`。from-source 端点一旦上线，
     局域网任一主机都能用 `file://` 读 GB10 本地文件（虽有白名单，但攻击面
     无谓扩大）。
   - 改默认 `--bind 127.0.0.1:8091`；compose 里 `ports` 映射改为
     `"127.0.0.1:8091:8091"`。需要对局域网暴露时显式 override。

### 目标接口契约

`POST /v1/audio/transcriptions/from-source`：

请求体 JSON：
```ts
{
  source: string,        // file:// abs path | http(s):// URL
  vad?: boolean,         // 同 Plan B
}
```

响应同 `/v1/audio/transcriptions`。

### 行为规则

| 输入 | 行为 |
|---|---|
| `file://` + 路径在白名单 + 文件存在 | 读本地文件 → ffmpeg → 推理 |
| `file://` + 路径不在白名单（canonical 后判定） | `403 forbidden_source` |
| `file://` + 文件不存在 | `404 not_found "source file not found"` |
| `file://` 路径含 `%` 编码字符 | `400 invalid_request "encoded file:// path not supported"` |
| `file://` Windows 风格（如 `file:///C:/...`） | `400 invalid_request "unsupported file:// path"` |
| `http(s)://` + 下载成功 + 体积合规 | 走 ffmpeg → 推理 → 删临时文件 |
| `http(s)://` + 下载超时 / 体积超限 | `400 invalid_request "fetch timeout/too large"`（临时文件已删） |
| `--source-allowlist` 未配置且 `from-source` 被调用 | `503 endpoint_disabled "from-source disabled; configure --source-allowlist"` |
| source scheme 不是 file/http/https | `400 invalid_request "unsupported source scheme"` |

### 禁止事项

- 不接受不带 scheme 的裸路径（必须 `file://`，避免歧义）
- 不允许 `file://` 跨链接跳出白名单（必须 canonical + prefix check）
- 不为 HTTP 下载实现重试 / 续传（调用方自己负责）
- 不缓存下载结果（每次重新下载，缓存是调用方的事）

### 测试要求

- `tests/from_source.rs`：
  - `test_file_in_allowlist`：写一份 test wav 到 tmp dir，启动 server with
    `--source-allowlist /tmp/`，调 `from-source` → 200 OK
  - `test_file_outside_allowlist`：路径在白名单外 → 403
  - `test_file_traversal`：`/tmp/../etc/passwd` → 403（canonical 后不在白名单）
  - `test_file_url_encoded_rejected`：`file:///tmp/%2e%2e/etc/passwd` → 400
    `encoded file:// path not supported`（防绕过 canonical 的二次解码攻击）
  - `test_endpoint_disabled_without_allowlist`：不配 `--source-allowlist` → 503
  - `test_http_url_size_limit`：mock 一个超过 `--max-source-bytes` 的 HTTP
    流（chunked encoding，前 N MB 后服务器仍在写）→ 400，且临时文件已删
- 验证命令：`cd server/asr-server && cargo test`

### 完成条件

- [ ] 上述 5 个测试全绿
- [ ] GB10 上 zero 实测：`curl -X POST -H 'Content-Type: application/json' -d '{"source":"file:///home/fengqi/.config/zero/downloads/some.mp4"}' http://localhost:8091/v1/audio/transcriptions/from-source` 拿到文本
- [ ] `--bind` 默认值改为 `127.0.0.1:8091`；compose 里 `ports` 改为 `"127.0.0.1:8091:8091"`；启动日志打印实际监听地址
- [ ] `README.md` 新增 "from-source 端点" 章节（白名单配置 + curl 样例 + 已知限制更新），并在末尾"调用样例"加一条 from-source 的 curl/Python 示例

---

## 4. 风险与待定项

| 风险 | 评估 | 处置 |
|---|---|---|
| ffmpeg 镜像体积膨胀 60 MB | 可接受（GB10 磁盘充足，asr-server 本来就是 opt-in profile） | 不处理 |
| `Mutex<OfflineRecognizer>` 串行解码使 VAD 多段累积延迟 | 1 分钟视频切 10 段 → 10 倍 sherpa 延迟 | 不在本 plan 处理；后续 recognizer pool 单独立项 |
| 白名单配置错误导致 zero 调不通 from-source | 启动日志 warn + endpoint 503 已经显式提示 | 文档强调 |
| SenseVoice vs Whisper 选型 | SenseVoice 对中文短视频 ITN 友好，Whisper-turbo 多语种稳；本 plan 不改选型 | 模型切换是另一个 plan |
| compose 是否暴露 asr-server 端口 | 当前 asr-server 在 `profiles: [asr-server]`，外部需要时手动 up | Plan C 已固化：默认 `--bind 127.0.0.1:8091` + compose `ports: ["127.0.0.1:8091:8091"]`；要对局域网开放需显式 override |

---

## 5. 调用方对接（供 zero 一侧参考，不在本 plan 范围）

zero 的 douyin skill 远期 MVP-4（视频 → 文本）期望调用方式：

```
zero 在 GB10 调下载工具 → mp4 落到 /home/fengqi/.config/zero/downloads/<aweme_id>.mp4
                        ↓
zero 调 douyin_transcribe(aweme_ids)  ← 新工具
                        ↓
HTTP POST http://localhost:8091/v1/audio/transcriptions/from-source
  body: {"source":"file:///home/fengqi/.config/zero/downloads/<aweme_id>.mp4","vad":true}
                        ↓
asr-server 拿到 segments[]，zero 持久化到博主知识包
```

asr-server 一侧需要（已在 Plan C 完成条件中固化）：
- 启动参数加 `--source-allowlist /home/fengqi/.config/zero/downloads/`
- 监听 `127.0.0.1:8091`（Plan C 默认值已改）
- zero 端写新工具 `douyin_transcribe`，参数 `aweme_id` / `vad`

zero 一侧的 plan 在 zero 仓库另立。
