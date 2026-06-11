# streaming-speech 全链路追踪接入设计

> 创建 2026-06-03。把 streaming-speech 的语音能力（ASR 转写 / TTS 合成）接入
> **trace-hub** 全生命周期追踪体系（`D:\git\trace-hub`，见其 `docs/DESIGN.md`）。
> 体系要点：W3C `traceparent` 传播 + `custom-utils` trace 客户端（异步非阻塞推送）+
> body-first（请求/响应/转写文本作为一等公民进详情）+ 节点异构 summary/detail。

## 1. 背景：本仓的三个语音组件

| 组件 | 形态 | 端口 | 谁调它 |
|---|---|---|---|
| `server/asr-server`（Rust, axum） | OpenAI 兼容转写 HTTP | `:8091` | **douyin** process worker（下载+ASR）；orchestrator |
| `server/orchestrator`（Rust, axum） | WebSocket 实时 ASR + LLM 优化/翻译 | `:8090` | 桌面端 / 发音教练实时链路 |
| `server/tts`（**Python**, CosyVoice 容器） | TTS 合成 HTTP（`{text,voice_id}`→WAV） | 容器端口 | **zero** 英语教练 `TtsClient` |

> ASR 推理在 asr-server 进程内（sherpa-onnx，`server/asr-server/src/main.rs:591` `recognizer.decode()`）。
> TTS 推理在 Python CosyVoice 容器内（`server/tts/cosy_server.py`），**非 Rust**。

## 2. 目标 / 非目标

**目标**：
- ASR 转写、TTS 合成纳入**同一条 trace**，挂到 zero 发起的生命周期树下。
- asr-server 内部拆出 `audio_decode` / `vad_segment` / `asr_decode`(sherpa 推理) 子 span，定位耗时瓶颈。
- 转写文本 / 合成文本作为 body 进详情，按 trace_id 与全链路其它节点互通。

**非目标（分期）**：
- Python CosyVoice 容器**内部**埋点留 Phase 3；本期 TTS span 由调用方（zero）记（caller-side），已足够看到时延+文本。
- orchestrator WS 实时链路留 Phase 2（流式会话建模较复杂）。

## 3. 在 trace 树里的位置

ASR/TTS 是**同步请求/响应**（不像闹钟跨异步），故用 `continued`/child（同 `trace_id`），**不用 span-link**。

```
trace（zero 起点）
├─ agent_task (zero, orchestrate hook)
│   └─ douyin process 提交 …（跨异步，douyin_done）
│        └─ [douyin worker 同步调用] asr_transcribe (asr-server)      ← 本设计 Phase 1
│             ├─ audio_decode  (ffmpeg/wav 解码)
│             ├─ vad_segment   (silero_vad 切段，vad=true 时)
│             └─ asr_decode ×N (sherpa 推理；详情含分段转写文本)
├─ llm_call ×N (zero←nova, 含 body)
└─ tts (zero TtsClient → cosyvoice)                                   ← 本设计 Phase 2（caller-side）
     └─（Phase 3 可选）cosyvoice 内部 synth span
```

## 4. 传播契约（谁注入 / 谁提取）

| 边界 | 载体 | 注入方（已具备接缝） | 提取方 |
|---|---|---|---|
| douyin worker → asr-server | `traceparent` 请求头 | douyin `process.rs:565` 发 ASR 请求处 `.header("traceparent", tp)`；tp 来自 worker 读 `<task_id>.trace` 侧文件（已实现）/ `job.traceparent` | **asr-server** handler 加 `HeaderMap` extract |
| zero TtsClient → cosyvoice | `traceparent` 请求头 | zero `tts_client.rs:81` 发 TTS 请求处注入；ctx 来自本轮 turn/session trace | （本期不强制）Python 容器；本期由 zero caller-side 记 span |
| 桌面端 → orchestrator WS | `traceparent`（WS 升级请求头 / hello 帧字段） | 客户端 | orchestrator（Phase 2） |

> 关键：ASR/TTS 调用方都已能拿到 traceparent（douyin 侧文件机制已实现；zero 进程内有 turn trace）。
> 故本仓改动集中在**被调用方提取 + 记 span**（asr-server），加调用方两行注入。

## 5. Phase 1 —— asr-server 接入（本仓主改动）

### 5.1 依赖（option B，与 trace-hub 体系一致）—— ✅ 已实现

根 `Cargo.toml` `[workspace.dependencies]` 增（path 为本地开发态，发布前改 git-tag）：
```toml
# trace-hub 接入：custom-utils trace 客户端（异步非阻塞推送 span+body 到 trace-hub）。
custom-utils = { path = "../custom-utils", default-features = false, features = ["trace"] }
```
`server/asr-server/Cargo.toml` 加 `custom-utils = { workspace = true }`。

实际落地（与原设计一致 + 两处统一改造，见 §11）：
- **reqwest 全仓统一到 0.13**（src-tauri / asr-server / orchestrator），与 custom-utils trace
  客户端同源，避免 workspace 里 0.12/0.13 双版本。注意 0.13 改了 TLS feature 名：
  旧 `rustls-tls` → `rustls`（0.13.3 自带 webpki 根；0.13.4+ 需另加 `webpki-roots`）。
- **orchestrator 并入根 workspace**（移除其 `[workspace]` 脱钩表、删独立 `Cargo.lock`），
  以便统一 lockfile 并为 Phase 3 备好 `[workspace.dependencies]`。连带需把其 `rusqlite`
  从 0.32 升到 0.38——否则与 src-tauri（经 deadpool-sqlite 拉 rusqlite 0.38）产生
  `libsqlite3-sys`（`links="sqlite3"`）冲突，同 workspace 只允许一份。
> custom-utils 的 `trace` feature 经其自身 path 拉 `trace-model`（与 trace-hub 共享契约）。
> ⚠ **此依赖方式只在本机开发态成立**；Docker 部署有额外前置，见 §11。

### 5.2 init（仅当设 `TRACE_HUB_ENDPOINT` 时启用，未设零影响）

`server/asr-server/src/main.rs:110` 起 `tracing_subscriber` 旁：
```rust
if let Ok(ep) = std::env::var("TRACE_HUB_ENDPOINT") {
    custom_utils::trace::init(custom_utils::trace::TraceConfig::new(ep, "asr-server"));
}
```
（须在 tokio 运行时内调用；asr-server 是 `#[tokio::main]`，OK。）

### 5.3 入站提取 + 根 span

两个 handler 加 `HeaderMap` 参数，提取 traceparent，作本服务这棵子树的根：
- `transcribe()`、`from_source()`（注意 `HeaderMap` 须在消费 body 的 `Multipart`/`Json` 之前）：
```rust
async fn from_source(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,                 // ← 新增（在 Json 之前）
    Json(req): Json<FromSourceRequest>,
) -> Result<Json<TranscriptionResponse>, ApiError> {
    let ctx = trace_root(&headers); // = extract_traceparent(...).map(|r| r.child()).unwrap_or(root())
    let t0 = trace::now_ms();
    // … 现有处理 …，把 ctx 串进 decode/识别，期间用 ctx.child() 记子 span（见 5.4）
    // 末尾：emit_span(span=ctx, kind="asr_transcribe", start=t0, end=now, summary, body=全文)
}
```
> **用 `.child()` 不用 `continued()`**：ASR 是同步请求/响应（非跨异步续接），`extract_traceparent`
> 返回的是「远端当前 span」，本地 `.child()` 出的 span 其 `parent_span_id` 即远端 span，语义正确。
> `continued()` 是给一次性异步回调（如闹钟到点）准备的。实现里抽了 `trace_root()` 辅助封装这步。

### 5.4 内部子 span（耗时拆解）

asr-server 的 trace 客户端是**显式构造 `SpanRecord` + `record_span` 入队**（非 `tracing` span 传播），
故 `spawn_blocking`（`:543`）跨线程无碍——在阻塞推理前后取 `now_ms()` 即可。

| 子 span kind | 记录点（文件:行） | 关键时段 |
|---|---|---|
| `audio_decode` | `decode_any()` `:417`（WAV 快路径 / ffmpeg 转码 `:421`，可能 >30s） | 解码耗时 |
| `vad_segment` | `vad::segment()` `:577`（vad=true 时） | silero_vad 切段 |
| `asr_decode` | `recognize()` 的 `recognizer.decode()`（★最有价值） | sherpa **逐段**推理（拍板：每段一条 span，detail=该段文本）|

子 span 的 `parent_span_id = root.span_id`（用 `ctx.child()` 生成）。推理在 `Mutex<OfflineRecognizer>` 串行，
可选记一个 `asr_wait`（拿锁前等待）子 span 暴露争用（本期未做）。

> **实现（已落地）**：抽了 `recognize_traced()` 包住单段识别 + 记一条 `asr_decode`；
> `transcribe_blocking()` 的 VAD 多段循环逐段调它（`seg_index`/`seg_dur_s`/`decode_ms`/段文本）。
> `recognize_long()` 内部对超 30s 段再切窗只作为 detail 级细节，不再单独开一级 span。
> 所有子 span 的 payload 构造都包在 `trace::enabled()` 判断内（见 §11 的 P1）。

### 5.5 节点字段（异构 summary/detail；body-first）

| kind | summary（小，树上显示） | detail / body（点击拉，trace-hub 懒加载+截断） |
|---|---|---|
| `asr_transcribe` | `{model, vad, source_kind(file/http), text_len, segments_count, total_ms}` | detail `{source}`；**response_body = 全文转写**（ASR 的「body」） |
| `audio_decode` | `{fmt, fast_path(bool), decode_ms}` | detail `{source}` |
| `vad_segment` | `{segments_count, vad_ms}` | detail `{boundaries:[{start,end}]}` |
| `asr_decode` | `{seg_index, seg_dur_s, decode_ms, text_len}` | **response_body = 该段文本** |

## 6. Phase 2 —— TTS（caller-side，在 zero 记）—— ✅ 已落地

Python CosyVoice 容器本期不改。TTS span 由 zero 侧记录，实际埋点位置**比设计稿上抬了一层**：
- traceparent 注入在 `bridge-claw/src/tts_client.rs:99`（`request_tts` 出站请求头），
- caller-side `record_span(kind="tts")` 在 `bridge-claw/src/tts_synthesize_tool.rs:101`
  调 `record_tts_span()`（定义同文件 :126-158），**而非** `request_tts` 内部。
  原因：`synthesize()` 命中本地缓存时根本不发请求（`tts_client.rs:72-74` 直接返回缓存路径），
  埋在 `request_tts` 里会漏记缓存命中那次「合成」；tool 层不管缓存命中与否都记一条，更准。

实现摘要（对照原设计稿）：
- `request_tts()` `:87`：注入 `traceparent`（为将来 Python 侧准备）——已实现；
```rust
// ctx 取自本轮 turn/session trace（见 6.1）
let now = trace::now_ms();
let resp = self.http.post(&self.endpoint)
    .header("traceparent", ctx.to_traceparent())
    .json(&json!({"text": text, "voice_id": voice})).send().await?;
let bytes = resp.bytes().await?;
trace::record_span(SpanRecord{
    kind:"tts", flow_name:None, /* ctx 字段 */,
    start_ms: now, end_ms: trace::now_ms(), status: Ok,
    summary: json!({"voice_id":voice, "text_len":text.len(), "bytes":bytes.len(), "dur_ms":..}),
    detail:  json!({"text":text}),     // 合成文本 = TTS 的「body」
    ..
});
```

### 6.1 trace 上下文怎么到达 TTS 工具 —— ✅ 已实现
TTS 由 `TtsSynthesizeTool` 在 turn 内执行，需拿到本轮 trace。bridge-claw 已维护 `session_id → TraceContext` 映射，工具经 `crate::session_trace(&context.session_id)` 反查 → `.child()`（见 `tts_synthesize_tool.rs:86-89`），缺失时跳过 `record_tts_span`（追踪未启用 / 无 session 映射场景安全降级）。`push_audio_tool` 也复用 `record_tts_span`。

## 7. Phase 3（可选）—— Python CosyVoice 内部 / orchestrator WS

- **cosyvoice**：Python 侧若要内部 `synth` span，用 OpenTelemetry Python 或一个极简 HTTP 推送到 trace-hub `/v1/spans`（trace-model 的 JSON 契约语言无关）。从请求头取 traceparent。
- **orchestrator WS 实时**：一次 `/stream` 会话 = 一棵 trace（或挂发起方 trace）；per-segment `asr_decode` + `optimize`/`translate`（orchestrator 调 vLLM `:752`）作 `llm_call` 子 span；WS 升级时从请求头/hello 帧取 traceparent。复杂度高，单独排期。

## 8. 调用方需配合的改动（跨仓清单）

| 仓 | 文件:行 | 改动 |
|---|---|---|
| github-commit-info(douyin) | `crates/douyin/src/process.rs:565` | ASR 请求 `.header("traceparent", tp)`；tp 来自 worker 已有的 `<task_id>.trace` 侧文件 / `job.traceparent`（侧文件机制已实现，见 trace-hub `instrumentation.md`） |
| zero | `crates/bridge-claw/src/tts_client.rs:99` + `tts_synthesize_tool.rs:101` | ✅ 已落地：traceparent 注入在 `tts_client.rs`；`record_span(kind="tts")` 在 `tts_synthesize_tool.rs`（上抬一层覆盖缓存命中场景）；§6.1 session→trace 映射用 `session_trace()` 反查 |

## 9. 施工计划

1. **Phase 1（本仓核心）—— ✅ 本机实现已落地**：asr-server 依赖+init+extract+`asr_transcribe`+
   三个子 span 已实现，本机 `cargo check`/`clippy -D warnings` 全绿。**尚未可部署**（见 §11）。
   待 douyin 注入 traceparent（跨仓，§8）后即可在抖音链路看到每段 ASR 时延 + 转写文本。
2. **Phase 2**：zero TtsClient caller-side `tts` span + session→trace 映射收敛。✅ 已在 zero 仓落地（埋点位置实际在 `tts_synthesize_tool.rs` 而非 `tts_client.rs`，原因见 §6）。
3. **Phase 3（可选）**：cosyvoice Python 内部 span；orchestrator WS 实时链路。orchestrator 已并入
   workspace，依赖接法届时同 asr-server（但同样受 §11 的 Docker 上下文约束）。

## 10. 风险 / 待定

- **spawn_blocking 内记 span**：✅ 已核实安全——客户端 `enqueue` 用 `try_send`（`custom-utils/src/util_trace/client.rs`），
  不依赖 tokio 运行时上下文，blocking 线程上直接调用即可（原 P3 风险消除）。
- **「未设零影响」**：✅ 已落实——`init` 仅在 `TRACE_HUB_ENDPOINT` 存在时调用；所有 span 的
  `summary`/`detail`/`response_body` 构造均包在 `trace::enabled()` 判断内，关闭时不做 JSON/clone，
  仅多几次廉价 `now_ms()`（原 P1：避免「关闭仍序列化全文」的隐性开销）。
- **body 体量**：转写全文 / 合成文本作 body，trace-hub 已按 `body_limit` 截断（默认 1MB）。
- **reqwest 版本**：已全仓统一 0.13（不再 0.12/0.13 并存）；详见 §11 的构建成本说明。
- **部署可达性**：streaming-speech 与 trace-hub 若不同机（如 g10），`TRACE_HUB_ENDPOINT` 要网络可达；未设则该服务零追踪、零影响。
- **option B path 依赖**：与其它仓一致，发布前移除 path、改回 registry/tag（需先发 trace-model、custom-utils）。**这对 GB10 Docker 构建是硬前置，见 §11**。
- **同步 ASR 的 trace_id 来源**：若 douyin 未注入 traceparent（无 trace 上下文），asr-server 回退 `root()` 起独立 trace（不孤儿，仍可单独看 ASR 内部）。

## 11. 部署前置 / 落地注意事项（✅ 均已处理，asr-server 可部署）

当前 GB10 部署模型（`scripts/release-server.ps1` 只把 `server/` 打 tar，每个服务以**自身子目录**
为 Docker 构建上下文独立编译，见 `server/compose.yaml` `build: ./asr-server`）下，以下三点已逐一落实：

1. **custom-utils 依赖来源 —— ✅ 已解决（registry）**。
   早期 `custom-utils` 是仓库**外**的 path 依赖且 `workspace = true` 在容器里无法解析（asr-server
   镜像只 COPY `server/asr-server/` 子目录，看不到根 workspace 与 sibling 仓）。
   **现 custom-utils `0.15.0` 已发布 crates.io，并把原 `trace-model` 内联进 `util_trace::model`**
   （公开路径不变：仍是 `custom_utils::trace::{SpanRecord, TraceContext, ...}`）。
   故 asr-server 改为**显式 registry 依赖** `custom-utils = { version = "0.15", default-features = false, features = ["trace"] }`，
   GB10 经 crates 镜像源（USTC / rsproxy）即可拉到，无需 vendoring。
   > 注：不放 `[workspace.dependencies]` + `workspace = true`——独立构建的容器解析不了；各 crate 写显式版本。

2. **reqwest 0.13 + rustls 的构建成本（GB10 arm64）—— ✅ 已加 cmake**。0.13 的 `rustls` feature 拉
   **aws-lc-rs**（不再是 0.12 rustls-tls 的 ring），其 `aws-lc-sys` 需 **cmake + C 工具链**构建。
   已在 asr-server / orchestrator 两个 Dockerfile 的 build 阶段加 `apt-get install -y cmake`。
   （如需规避 aws-lc-rs，可改 `rustls-no-provider` + ring provider；本期选直接装 cmake。）

3. **orchestrator 已并入根 workspace 的连带改动（已在本仓完成）**：
   - 移除 `server/orchestrator/Cargo.toml` 的 `[workspace]` 脱钩表；
   - 删除 `server/orchestrator/Cargo.lock`（改由根 workspace lock 管理）；
   - `server/orchestrator/Dockerfile` 不再 `COPY ... Cargo.lock`（容器内独立构建现场生成，同 asr-server）；
   - `rusqlite` 0.32→0.38（解 `libsqlite3-sys` links 冲突）。
   这些不影响 orchestrator 现有运行/协议，仅统一了开发态工程结构。

4. **运行期开关（compose）—— ✅ 已配**。trace-hub 是 GB10 **宿主进程**（监听 `0.0.0.0:9100`），
   不在 compose 内。asr-server 容器经 `host.docker.internal` 到宿主，故 `server/compose.yaml`
   asr-server 服务已加：
   - `TRACE_HUB_ENDPOINT=http://host.docker.internal:9100/v1/spans`
   - `extra_hosts: ["host.docker.internal:host-gateway"]`（同 orchestrator 连 vLLM 的方式）
   删掉该 env 即回到「零追踪、零影响」。trace-hub UI：`http://<gb10>:9100/`。

## 12. orchestrator 追踪接入详细设计（原 Phase 3 落地）

> 创建 2026-06-04。背景：zero 侧 trace 已联通（commit `a5d7b1b` zero-nova + `50621b1` custom-utils + zero 改 git rev 跨编通过、`zero/llm_call` span 上 trace-hub 已验证）。本节把原 §7 简略的 orchestrator 计划具体化，目标：**桌面端/教练实时链路也挂回 zero 起点的同一棵 trace**。
>
> Phase 1 的 asr-server 已上线；orchestrator 与 asr-server 在同一 compose 里互相 `depends_on`，
> 容器侧设计完全对齐，差异只在 **入站载体（WS vs HTTP）** 和 **出站对象（vLLM 而非 sherpa）**。

### 12.1 trace 在 orchestrator 里的位置

```
trace（zero 起点 / 桌面端起点二选一）
└─ ws_stream (orchestrator, 一次实时会话 = 1 棵子树根)
    ├─ asr_segment ×N (一段音频 → 转写)
    │   └─ asr_decode (调 asr-server `:9100` 内部 ws；带 traceparent 头)
    └─ optimize / translate (调 vLLM `:8085/v1` chat completions)
        └─ llm_call  (record_llm_call，含 vLLM 请求/响应 body)
```

- ASR 走容器内 ws（`ASR_WS=ws://asr:9100`），不是 HTTP，但 ws 升级请求头可塞 `traceparent`，被调 asr-server 端按 §5.3 同样模式提取。
- vLLM 调用走 HTTP（`VLLM_BASE=http://host.docker.internal:8085/v1`），vLLM 自身不感知 traceparent；orchestrator caller-side `record_llm_call` 拿到本次 req/resp body 即可，无下游续接。

### 12.2 入站 traceparent 的两种来源

| 入站方式 | 载体 | 说明 |
|---|---|---|
| 桌面端 → orchestrator WS 升级（路径 `/stream`） | **WS 升级 HTTP 头 `traceparent`** | 首选：和 HTTP handler 完全一致的 `HeaderMap` extract |
| 客户端无法改升级头时 | 首帧 hello JSON `{"traceparent":"..."}` | 兜底：解析首帧时 `extract_traceparent_str()` |

两条路径都没拿到时回退 `TraceContext::root()`——orchestrator 自身起一棵 trace（不孤儿，仍可看内部）。

### 12.3 代码改动清单

| 位置 | 改动 | 产出 span |
|---|---|---|
| `server/orchestrator/Cargo.toml [dependencies]` | 加 `custom-utils = { version = "0.15", default-features = false, features = ["trace"] }`（显式 registry，不走 `workspace=true`——和 asr-server 同理，容器独立构建） | — |
| `server/orchestrator/src/main.rs`（启动处） | `if let Ok(ep) = std::env::var("TRACE_HUB_ENDPOINT") { trace::init(TraceConfig::new(ep, "orchestrator")); }` | — |
| WS 升级 handler（axum `ws_handler` 之类） | 加 `HeaderMap` 参数 → `trace_root(&headers)` 拿 `ctx`；同时 hello 帧解析处给个 fallback；把 `ctx` 一路传到 session 状态 | `ws_stream` 根 span（会话结束时 emit） |
| ASR ws 客户端出站处（连 asr-server 处） | 升级请求头 `.header("traceparent", ctx.to_traceparent())`；每段识别完 `record_span(kind="asr_segment", ...)` | `asr_segment` ×N |
| 调 vLLM 的 reqwest 调用处（`:752` 那段） | 调用前 `let t0 = now_ms();` 调用后 `record_llm_call(LlmCall{ctx: ctx.child(), model, request_body, response_body, start_ms: t0, end_ms: now_ms(), status})` | `llm_call`（含 body）|

> ⚠ 所有 `record_*` payload 构造**包在 `trace::enabled()` 内**，未启用时零序列化代价（同 §5/§10 P1 原则）。

### 12.4 span 字段约定（summary/detail，body-first）

| kind | summary（树上） | body（详情） |
|---|---|---|
| `ws_stream` | `{session_id, segments:N, total_ms, asr_model, llm_model}` | detail `{client_addr}` |
| `asr_segment` | `{seg_index, dur_ms, text_len}` | response_body = 该段转写文本 |
| `llm_call`（同 zero 那套） | `{model, dur_ms}` | request_body / response_body 完整 JSON |

### 12.5 与 vLLM 的解耦

vLLM 是 GB10 主机进程（不在 compose 内、不感知 trace-hub），orchestrator caller-side 已能完整记录本次 LLM 调用——**不需要给 vLLM 加任何东西**。若将来要拆 prompt assembly / sampling 阶段，再在 orchestrator 内部加子 span。

### 12.6 不做的事（明确范围）

- 不改 vLLM
- 不改 asr-server 协议（asr-server 已在 ws 升级请求头按 `HeaderMap` 提取 traceparent，§5.3 模式直接复用）
- 不做 OTel/zipkin 转译——orchestrator 直推 trace-hub

## 13. 容器部署 runbook（asr-server + orchestrator）

> 当前 GB10 上 streaming-speech 的容器**都没在跑**（`docker ps` 验证；可能从未起过或被停掉）。
> 本节给出标准启动路径，把 asr-server 起来即可验证 §5 的 trace 接入，orchestrator 改完代码后同步起。

### 13.1 前置检查

```bash
ssh fengqi@192.168.0.68 'docker ps && docker compose version'
# 期望：可见 vllm 等容器；compose v2 可用
```

并确认 trace-hub 宿主进程在跑（默认 `~/.config/systemd/user/trace-hub.service`，监听 `:9100`）。

### 13.2 拉源码到 GB10

streaming-speech 在 GB10 上没源码挂载约定，按既有惯例：

```bash
# 本机
scp -r D:/git/streaming-speech/server fengqi@192.168.0.68:~/streaming-speech-server
# GB10
ssh fengqi@192.168.0.68 'ls ~/streaming-speech-server/compose.yaml && \
  cd ~/streaming-speech-server && \
  docker compose --profile asr-server build asr-server'
```

> 注：asr-server **默认不参与** `docker compose up -d`，因 `profiles: [asr-server]`。需要 `--profile asr-server` 才纳管，避免影响生产栈。

### 13.3 启动

```bash
# 只起 asr-server（不影响现有 asr/orchestrator/tts）
cd ~/streaming-speech-server && docker compose --profile asr-server up -d asr-server

# 看日志确认 trace init 行
docker logs -f $(docker ps -qf name=asr-server) 2>&1 | head -30
# 期望看到 asr-server 的 axum bind on 0.0.0.0:8091 + trace::init endpoint 行
```

trace endpoint 已经在 `compose.yaml` 里写死：
```yaml
- TRACE_HUB_ENDPOINT=http://host.docker.internal:9100/v1/spans
extra_hosts: ["host.docker.internal:host-gateway"]
```
未设此 env 则容器零追踪、零影响——`compose.yaml` 删 env 即关。

### 13.4 健康验证

```bash
# 容器内 8091 已 bind 0.0.0.0，宿主 127.0.0.1:8091 转发只本机可达：
ssh fengqi@192.168.0.68 'curl -s http://127.0.0.1:8091/v1/models | head -c 200'

# 跑一次转写打通端到端 trace：
ssh fengqi@192.168.0.68 \
  'curl -s -X POST http://127.0.0.1:8091/v1/audio/transcriptions \
     -F "file=@/home/fengqi/.config/zero/downloads/<some.wav>" \
     -F "model=sense-voice" \
     -H "traceparent: 00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"'

# 看 trace-hub：
ssh fengqi@192.168.0.68 \
  'sqlite3 ~/.config/trace-hub/spans.db "SELECT service, kind, COUNT(*) FROM span GROUP BY service, kind;"'
# 期望出现：asr-server | asr_transcribe / audio_decode / vad_segment / asr_decode
```

### 13.5 orchestrator 起容器（待 §12 代码改完后）

```bash
cd ~/streaming-speech-server && docker compose up -d orchestrator
# orchestrator 默认在主 profile 中，up -d 会一起带 asr（生产链路），不带 asr-server
```

orchestrator 容器 `compose.yaml` 当前还缺 `TRACE_HUB_ENDPOINT` env——§12 代码改完一同补：
```yaml
environment:
  - ORCH_BIND=0.0.0.0:8090
  - ASR_WS=ws://asr:9100
  - VLLM_BASE=http://host.docker.internal:8085/v1
  - VLLM_MODEL=gemma-4-26B-A4B-it
  - DATA_DIR=/data
  - TRACE_HUB_ENDPOINT=http://host.docker.internal:9100/v1/spans   # 新增
extra_hosts:
  - "host.docker.internal:host-gateway"
```

### 13.6 排错速查

| 现象 | 大概率原因 | 处理 |
|---|---|---|
| 容器起来但 trace-hub 无 span | env 没注入 / host.docker.internal 不通 | `docker exec <c> env \| grep TRACE`；`docker exec <c> curl http://host.docker.internal:9100/healthz` |
| `bind: address already in use :8091` | 旧实例残留 | `docker ps -a` 找到 stop+rm，或换 ports 映射 |
| asr-server 启动后日志无 `trace::init` 行 | TRACE_HUB_ENDPOINT 未设 | compose env 节确认；删 env 是设计性关闭，无报错 |
| 加了 `traceparent` 但 ASR 仍起独立 trace | 头名/格式不对 | 必须是小写 `traceparent`，值符合 W3C `00-{32hex}-{16hex}-{2hex}` |

## 参考
- trace-hub 体系设计：`D:\git\trace-hub\docs\DESIGN.md`、埋点指南 `instrumentation.md`。
- 契约：`custom_utils::trace`（原 `trace-model` 已内联：SpanRecord / TraceContext / W3C traceparent）。
- 调用方现状：douyin `crates/douyin/src/process.rs:565`（ASR 出站）、zero `crates/bridge-claw/src/tts_client.rs:81`（TTS 出站）。
