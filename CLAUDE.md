# CLAUDE.md

This file orients Claude Code (claude.ai/code) in this repository.

## Project Overview

**StreamSpeech** is a **thin Windows desktop client + GB10 server** stack for real-time
Chinese/multilingual speech transcription with optional LLM polish & translation.
(The repo started as a single-binary offline Tauri app; the current `main` is the
post-refactor thin-client form. See `docs/HANDOFF.md` for migration history.)

```
Windows desktop (Tauri/Rust)         GB10 (192.168.0.68, arm64+CUDA13, Ubuntu24, Docker)
  mic capture → WS upload                ├─ orchestrator  :8090  WS + SQLite + Web 管理台 + /api/*
                                         ├─ asr           :9100/:9101 (内部)  FunASR + 声纹门控
                                         ├─ vLLM (主机)   :8085  gemma-4-26B (润色/翻译)
                                         └─ asr-server    :8091  sherpa-onnx, OpenAI-兼容 HTTP (外部调用)
                                         └─ TTS bake-off  :8095/:8096 (CosyVoice2 / GPT-SoVITS,选型隔离)
```

**Authoritative docs** — read these before deep work:
- `docs/HANDOFF.md` — current status, what's done, what's next, key commits
- `docs/DEPLOYMENT.md` — full deploy/ops reference (ports, volumes, redeploy flow)
- `docs/redesign-architecture-overview.md` + `docs/protocol-draft.md` — design decisions, WS protocol
- `server/tts/README.md` — TTS bake-off runbook (independent track)

## Top-level layout

| Path | What lives there |
|---|---|
| `src-tauri/` | Tauri desktop client (Rust). Thin: mic + UI + clipboard + remote WS. **No** sherpa-onnx, **no** local models. |
| `src/` | React 19 + TypeScript + Vite + Tailwind front-end of the Tauri app. |
| `server/orchestrator/` | Rust/axum service on GB10: client WS termination + SQLite + Web admin + HTTP API. |
| `server/asr/` | Python FunASR container: streaming VAD + Paraformer/SenseVoice recognition + speaker gating + `/embed`. |
| `server/asr-server/` | Rust binary container: standalone sherpa-onnx behind OpenAI-compatible HTTP (`/v1/audio/transcriptions`). Opt-in via compose `profiles: [asr-server]`. |
| `server/tts/` | CosyVoice2 + GPT-SoVITS bake-off (independent compose, see its README). |
| `server/compose.yaml` | Production-stack compose (asr + orchestrator; asr-server profiled out). |
| `scripts/release-server.ps1` | One-shot GB10 deploy (tar → scp → compose build/up → smoke). |
| `docs/` | Architecture, protocol, deploy, handoff. |

## Workspace

Root `Cargo.toml` is a Cargo workspace with two members: `src-tauri` (desktop client)
and `server/asr-server` (Rust HTTP service). They are intentionally independent crates;
nothing is shared at the library level.

## Commands

### Desktop client (Windows)
```powershell
$env:REMOTE_ASR_URL = "ws://192.168.0.68:8090/stream"    # required; remote-only mode
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
.\scripts\release-server.ps1 -Service asr-server # external OpenAI-compat ASR (profile)
.\scripts\release-server.ps1 -NoBuild            # sync + up -d, skip rebuild
.\scripts\release-server.ps1 -SyncOnly           # push files only, don't touch containers
```
Smoke endpoints: `curl http://192.168.0.68:8090/api/stats` (orchestrator) /
`curl http://192.168.0.68:8091/healthz` (asr-server).

### Web admin
Browser → `http://192.168.0.68:8090/` (overview, history, voiceprints, runtime config).

## Communication patterns

- **Client ↔ orchestrator**: WebSocket `/stream` (protocol in `docs/protocol-draft.md`).
  Upstream 16 kHz PCM; downstream `segment` / `optimized` / `translated` events.
  Auto-reconnect lives in `src-tauri/src/commands/remote.rs`.
- **orchestrator ↔ asr**: internal WS `ws://asr:9100`.
- **orchestrator ↔ vLLM**: HTTP `host.docker.internal:8085/v1` (vLLM runs on host, not in compose).
- **asr-server**: standalone HTTP `/v1/audio/transcriptions` (OpenAI Audio API shape) for
  external callers; not used by the desktop client.
- **Runtime config**: many settings live in orchestrator's SQLite `config` table and are
  edited from the Web admin (`asr.model`, `asr.spk_threshold`, `asr.sentence_gap_ms`,
  `asr.gate_to_enrolled`, `vllm.model`, `vllm.base`, `llm.optimize_prompt`,
  `llm.translate_prompt`). asr polls `/api/asr-config` every ~15s — most changes apply
  without redeploy.

## Important constraints

- **Models live on the server**, not in the client. GB10 paths: `~/funasr-prep/models/`
  (FunASR + CAM++ speaker model), `~/asr-server-models/` (sherpa-onnx models).
- **GB10 network gotchas**: GitHub direct is **unreliable** (clone/download often times out);
  use `hf-mirror.com` for HuggingFace assets and ModelScope (`~/ms_venv/bin/modelscope`)
  for ModelScope assets. crates.io is fronted by `rsproxy.cn` in every server Dockerfile.
- **arm64 + CUDA13**: sherpa-onnx upstream CUDA prebuilts stop at CUDA 12.x, so the
  current asr-server image is **CPU-only**. GPU build for asr-server is a follow-up that
  requires building sherpa-onnx native libs from source for arm64+CUDA13.
- **Production-stack autostart**: `asr` and `orchestrator` use `restart: unless-stopped`
  so GB10 reboots bring the service back automatically. `asr-server` is opt-in (compose
  profile), so it does not auto-start with the production stack — bring it up explicitly.
- **vLLM is a host process**, not in compose. It's maintained out-of-band; the orchestrator
  reaches it via `host.docker.internal:8085`.
- **Deploy = scp + rebuild**, not git pull. GitHub is blocked on GB10, and `~/server/`
  is not a git checkout. Use `scripts/release-server.ps1` for sync.

## Deploy workflow (server-side iteration)

1. Edit code locally under `server/*/`.
2. `cd src-tauri && cargo check` (if Rust) or `python -c 'import server.asr.app'` smoke.
3. `.\scripts\release-server.ps1 -Service <asr|orchestrator|asr-server>`.
4. Watch smoke output; on failure, `ssh fengqi@192.168.0.68 'docker compose logs --tail=80 <svc>'`.

The script uses Windows `System32\tar.exe` (bsdtar) explicitly — Git Bash's GNU tar
mis-parses Windows paths like `C:\...` as host:path.
