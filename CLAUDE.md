# CLAUDE.md

This file orients Claude Code (claude.ai/code) in this repository.

## Project Overview

**StreamSpeech** is a **thin Windows desktop client + GB10 server** stack for real-time
Chinese/multilingual speech transcription with optional LLM polish & translation.
(The repo started as a single-binary offline Tauri app; the current `main` is the
post-refactor thin-client form. See `docs/HANDOFF.md` for migration history.)

```
Windows desktop (Tauri/Rust)         GB10 (192.168.0.68, arm64+CUDA13, Ubuntu24, Docker)
  mic capture → WS upload                ├─ orchestrator   :8090  WS + SQLite + Web 管理台 + /api/*
                                         ├─ asr            :9100 (内部 WS) | 127.0.0.1:9101 (HTTP /embed + /transcribe)
                                         │                FunASR + 声纹门控
                                         ├─ vLLM (主机)    :8085  gemma-4-26B (润色/翻译)
                                         └─ TTS bake-off   :8095/:8096 (CosyVoice2 / GPT-SoVITS,选型隔离)
```

> 注：曾经的 `asr-server`（:8091 sherpa-onnx OpenAI 兼容 HTTP，供外部调用）
> **已物理退役**——先迁至 toolkit 仓 `crates/asr-server`，2026-06 又因与本仓 FunASR
> 能力重叠（且 FunASR 中文准确率/热词/声纹/模型丰富度全面占优）从 toolkit 整 crate
> 删除。外部离线转写**统一改走本仓 FunASR 的 `/transcribe` 端点**（`server/asr` 的
> `:9101`，multipart 上传 wav/mp3/mp4 → `{text, segments[{t_start,t_end,text}], model}`）。
> 详见 [`server/asr-server/MOVED.md`](server/asr-server/MOVED.md)。

**Authoritative docs** — read these before deep work:
- `docs/HANDOFF.md` — current status, what's done, what's next, key commits
- `docs/DEPLOYMENT.md` — full deploy/ops reference (ports, volumes, redeploy flow)
- `docs/redesign-architecture-overview.md` + `docs/protocol-draft.md` — design decisions, WS protocol
- `docs/asr-transcribe-api.md` — FunASR `/transcribe` HTTP API contract (multipart in / JSON out, for toolkit + any外部消费方)
- `server/tts/README.md` — TTS bake-off runbook (independent track)

## Top-level layout

| Path | What lives there |
|---|---|
| `src-tauri/` | Tauri desktop client (Rust). Thin: mic + UI + clipboard + remote WS. **No** sherpa-onnx, **no** local models. |
| `src/` | React 19 + TypeScript + Vite + Tailwind front-end of the Tauri app. |
| `server/orchestrator/` | Rust/axum service on GB10: client WS termination + SQLite + Web admin + HTTP API. |
| `server/asr/` | Python FunASR container: streaming VAD + Paraformer/SenseVoice/Whisper + speaker gating + HTTP `/embed`（声纹注册）+ HTTP `/transcribe`（离线整段，给同机 toolkit 抖音管线 multipart 上传 mp4 字节）。 |
| `server/asr-server/` | **已退役**（2026-06）。仅留 `MOVED.md` 记录历史 + 指引新入口（同仓 `server/asr` 的 `/transcribe`）。 |
| `server/tts/` | CosyVoice2 + GPT-SoVITS bake-off (independent compose, see its README). |
| `server/compose.yaml` | Production-stack compose (asr + orchestrator). |
| `scripts/release-server.ps1` | One-shot GB10 deploy (tar → scp → compose build/up → smoke). |
| `docs/` | Architecture, protocol, deploy, handoff. |

## Workspace

Root `Cargo.toml` is a Cargo workspace with two members: `src-tauri` (desktop client)
and `server/orchestrator` (Rust/axum GB10 service). They are intentionally independent
crates; nothing is shared at the library level. (`server/asr-server` was a former member,
later moved to toolkit, then physically retired in 2026-06 — see
`server/asr-server/MOVED.md` for the full timeline and where ASR lives now.)

## Commands

### Desktop client (Windows)
```powershell
# Connection URL is configured in-app (control panel → 连接地址 dropdown).
# Built-in default `ws://192.168.0.68:8090/stream` is always shown; users add
# custom presets via the dropdown, persisted in the client's local SQLite
# (`remote.url` / `remote.url_presets`). The old REMOTE_ASR_URL env var is gone.
npm run dev                              # Vite dev server + Tauri window
npm run build                            # tsc + Vite + Tauri release bundle (NSIS .exe)
cd src && npx tsc --noEmit               # front-end type check
cd src-tauri && cargo check              # back-end compile check
cd src-tauri && cargo test               # inline unit tests
```

### Server (from repo root)
```powershell
.\scripts\release-server.ps1                     # default: sync + rebuild + restart asr + orchestrator
.\scripts\release-server.ps1 -Service asr        # only asr
.\scripts\release-server.ps1 -NoBuild            # sync + up -d, skip rebuild
.\scripts\release-server.ps1 -SyncOnly           # push files only, don't touch containers
```
Smoke endpoint: `curl http://192.168.0.68:8090/api/stats` (orchestrator).

### Web admin
Browser → `http://192.168.0.68:8090/` (overview, history, voiceprints, runtime config).

## Communication patterns

- **Client ↔ orchestrator**: WebSocket `/stream` (protocol in `docs/protocol-draft.md`).
  Upstream 16 kHz PCM; downstream `segment` / `optimized` / `translated` events.
  Auto-reconnect lives in `src-tauri/src/commands/remote.rs`.
- **orchestrator ↔ asr**: internal WS `ws://asr:9100`.
- **orchestrator ↔ vLLM**: HTTP `host.docker.internal:8085/v1` (vLLM runs on host, not in compose).
- **Runtime config**: many settings live in orchestrator's SQLite `config` table and are
  edited from the Web admin (`asr.model`, `asr.secondary_model`, `asr.spk_threshold`,
  `asr.sentence_gap_ms`, `asr.gate_to_enrolled`, `vllm.model`, `vllm.base`,
  `llm.optimize_prompt`, `llm.translate_prompt`). asr polls `/api/asr-config` every
  ~15s — most changes apply without redeploy.
- **合并链模式（断链边界 = 客户端「合并间隔」`merge_window_ms`，主路径）**: 客户端把
  `merge_window_ms`(默认 3000ms,与复制 stitch 同一个值)随 `hello` 发给服务端。
  orchestrator 把**相邻**(`t0 - 上段t_end < 合并间隔`)VAD 段的**原始 ASR** 累积进一条
  「合并链」,间隔超阈值即开新链(`relay_asr` 里的 `ActiveChain`,边界与客户端
  `next_clipboard_text` 的 `< window` 合并条件镜像)。链 id 复用为该链首段的 `SEG_ID`,
  `segment` / `optimized` / `translated` 事件全部以 `ref=chain_id` 回发,**整条链一并润色**
  (输入是链的累积原始文本,**不再注入 `segments_context_before` 历史上文**),客户端按
  id 整体替换。这从根上断开了"润色回显被当上文喂回 → 再回显"的滚雪球,是"复制重复"
  的根因修复。同链随每个新段重复触发润色,并发结果用 `opt_emitted`/`tr_emitted`
  的 latest-wins(输入字符数不短于已发出才放行)防乱序覆盖。合并模式下**次模型对比
  被禁用**(逐段对照与链语义冲突)。`merge_window_ms=0` 关闭合并(与客户端关 stitch
  一致),回到下面的旧上下文路径。改间隔需重连服务端才生效(hello 时读一次,语义同
  `want_*`;客户端复制 stitch 仍每事件 live 读取)。
- **润色 LLM 带历史上下文（旧路径，仅 `merge_max_chars=0` 时生效）**: orchestrator 调润色
  前会把近 20s 内的历史已优化文本作为 user message 的「【近期上文，仅供参考，禁止
  输出】」段拼进去 (`build_optimize_user_msg` + `segments_context_before`)。prompt 写了
  禁止输出上文,但 LLM **不保证遵守**——Optimized 事件文本可能含前段片段。**任何在
  客户端做拼接/stitch/合并的功能(如 `next_clipboard_text` 自动复制链)都必须考虑
  去重**,否则会出现"复制重复"。次模型 re-polish 也走同一路径,同样带上下文。
  (合并链模式开启后此路径不再走,但客户端 `join_dedup` 仍保留:既给旧路径兜底,
  也用于合并模式下**跨链** stitch 的去重。)
  - **现行去重方式**:`src-tauri/src/commands/remote.rs` 的 `join_dedup` / `strip_overlap_prefix`
    按 char 找 head 后缀 与 tail 前缀的最长重叠(`MIN_OVERLAP_CHARS=2` 避免单字
    误判,`MAX_OVERLAP_CHARS=200` 防退化);有重叠续接不插空格,无重叠空格拼,
    tail 全含返 head 不追加。`AutoCopyAccum.prefix` 字段独立记录"当前 ref 之前的
    链上累计",同 ref 重发用 `join_dedup(prefix, new_text)` 替换本段贡献保链
    (回归锁:`same_ref_reemit_preserves_chain_prefix` 测试)。
  - **已知误判方向**:用户连说两次的短词(如「你好你好」)可能被当成 LLM 漏出的
    上文剥掉,表现为"少字"。中文典型句长 5+ char,实际碰撞低,2-char 阈值是
    刻意选的折中。若日后出现"少字"症状,先怀疑 `join_dedup`:把 `MIN_OVERLAP_CHARS`
    调到 4 试,或加日志打 head 尾/tail 头/剥掉 char 数。详见
    `~/.claude/projects/D--git-streaming-speech/memory/polish_llm_context_contract.md`。
- **次模型对比识别**: 桌面端 ControlPanel 开关「次模型对比识别」(默认关) →
  hello.want_secondary=true。orchestrator 给 asr 发 `{type:config,want_secondary}`,
  asr 在 finalize() 主段 emit 之后 run_in_executor 跑 `asr.secondary_model`,以
  `{type:secondary,t_start,t_end,text,kind}` 回发。orchestrator 按 `(t0,t1)` 配回
  主段 id,落库 `segments.secondary` 列,发 `ServerEvent::Secondary { ref, text, kind }`。
  桌面端 SegmentCard 在原文行下方紧贴一个带"次模型"标签的灰底行 + hover 复制按钮。
  次模型只跑识别 (不进润色/翻译),仅供对比中文识别精度。

## Important constraints

- **Models live on the server**, not in the client. GB10 path: `~/funasr-prep/models/`
  (FunASR + CAM++ speaker model). (sherpa-onnx models for the migrated asr-server now
  live with the toolkit deployment, not this repo.)
- **GB10 network gotchas**: GitHub direct is **unreliable** (clone/download often times out);
  use `hf-mirror.com` for HuggingFace assets and ModelScope (`~/ms_venv/bin/modelscope`)
  for ModelScope assets. crates.io is fronted by `rsproxy.cn` in every server Dockerfile.
- **Production-stack autostart**: `asr` and `orchestrator` use `restart: unless-stopped`
  so GB10 reboots bring the service back automatically.
- **vLLM is a host process**, not in compose. It's maintained out-of-band; the orchestrator
  reaches it via `host.docker.internal:8085`.
- **Deploy = scp + rebuild**, not git pull. GitHub is blocked on GB10, and `~/server/`
  is not a git checkout. Use `scripts/release-server.ps1` for sync.

## Deploy workflow (server-side iteration)

1. Edit code locally under `server/*/`.
2. `cd src-tauri && cargo check` (if Rust) or `python -c 'import server.asr.app'` smoke.
3. `.\scripts\release-server.ps1 -Service <asr|orchestrator>`.
4. Watch smoke output; on failure, `ssh fengqi@192.168.0.68 'docker compose logs --tail=80 <svc>'`.

The script uses Windows `System32\tar.exe` (bsdtar) explicitly — Git Bash's GNU tar
mis-parses Windows paths like `C:\...` as host:path.
