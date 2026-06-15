# Plan 3: toolkit 联动（client + douyin + toolkit-server 代理）

## 前置依赖

Plan 2（清洗服务 + HTTP 契约）—— 本 Plan 仅依赖其 `POST /clean` 契约，不依赖其实现细节。

> **跨仓说明**：本 Plan 的全部代码改动落在 **toolkit 仓**（`D:\git\github-commit-info`），
> 不改本仓（streaming-speech）任何代码。两仓以 Plan 2 §HTTP 契约为唯一接口，独立部署——
> 与现有 `asr-client ↔ FunASR /transcribe` 的关系完全相同。
>
> **范围边界**：zero-desktop 桌面端入口拆到 [Plan 4](audio-cleanup-plan-4.md)，本 Plan **不含**
> 任何 zero-desktop 改动——避免 Tauri 桌面侧改动阻塞服务端联动的评审与交付。

## 任务目标

在 toolkit 仓新增 `audio-clean-client` crate（照抄 `asr-client`），让**两类服务端消费方**接入
清洗服务：douyin 管线前置清洗（去 BGM 提升 ASR）、toolkit-server 代理路由。

## 执行范围

- **必须新增（toolkit 仓）**：`crates/audio-clean-client/`（lib + 类型 + 测试）。
- **必须修改（toolkit 仓）**：`crates/douyin/src/process.rs`（接入前置清洗）、douyin `Job` 结构、
  toolkit-server audio 路由、toolkit `CLAUDE.md` 服务清单。
- **禁止修改**：本仓（streaming-speech）任何文件；`asr-client` crate 现有公开接口；
  **zero-desktop 任何文件（属 Plan 4）**。

## 目标接口契约（`audio-clean-client`，照抄 asr-client 形状）

```rust
pub struct AudioCleanClient { http: reqwest::Client, clean_base_url: String }

pub enum PauseMode { Drop, Duck, Off }
pub enum Level { Gentle, Balanced, Aggressive }
pub enum AudioFormat { Wav, Mp3, Flac }

pub struct CleanOpts {            // 镜像 Plan 2 §POST /clean multipart 字段
    pub separate: bool,
    pub denoise: bool,
    pub pause: PauseMode,
    pub level: Level,
    pub loudness: Option<f32>,    // None = off
    pub sr: u32,
    pub format: AudioFormat,
}

pub struct CleanedAudio {         // 响应体二进制 + 从响应头解析的元数据
    pub bytes: Vec<u8>,
    pub stages: Vec<String>,      // X-Cleanup-Stages
    pub in_lufs: f32,             // X-Cleanup-In-LUFS
    pub out_lufs: f32,            // X-Cleanup-Out-LUFS
}

impl AudioCleanClient {
    pub fn with_client(http: reqwest::Client, clean_base_url: &str) -> Self;
    pub async fn clean_path(&self, path: impl AsRef<Path>, opts: CleanOpts) -> anyhow::Result<CleanedAudio>;
    pub async fn clean_bytes(&self, bytes: Vec<u8>, file_name: impl Into<String>,
                             mime: impl AsRef<str>, opts: CleanOpts) -> anyhow::Result<CleanedAudio>;
}
```

### base / 路径契约（**消除歧义，统一约定**）

| 项 | 约定 |
|---|---|
| 字段名 | **`clean_base_url`**（不叫 `base`、不叫 `clean_url`） |
| 取值 | **不含** `/clean` 路径段，如 `http://127.0.0.1:8097` |
| 默认 | `http://127.0.0.1:8097`（库层便利默认，平行于 asr-client 的 `:9101`） |
| 拼接 | 客户端**内部固定**拼 `/clean`；调用方只给 base，绝不自己带 `/clean` |
| **防错** | `with_client` 内**主动 trim** 掉末尾的 `/` 与 `/clean` 段（沿用 asr-client `strip_suffix("/transcribe")` 的容错先例），保证即便误传 `.../clean` 也不会拼成 `/clean/clean`；单测覆盖 |

> 这条专门修正旧设计里「`base=:8097` 又 `clean_url=:8097/clean`」的双路径冲突，且不只靠
> 文档禁止——客户端代码层 trim 兜底。

## Agent 执行步骤（toolkit 仓）

1. 新增 `crates/audio-clean-client/`：按上述契约实现；`with_client` 内对入参 `clean_base_url`
   **trim 末尾 `/` 与 `/clean`**（防 `/clean/clean`）；multipart 手工拼装
   （`Form::new().part("audio", Part::bytes(...))` + 各 `CleanOpts` 字段 `.text(...)`）；
   请求时内部拼 `/clean`；超时常量 `CLEAN_TIMEOUT = 600s`（具名常量）；
   错误分类沿用 asr-client（HTTP 错误带响应前 200 字、解析失败带上下文、**不自动重试**）。
   依赖按 workspace（`{ workspace = true }`）引用。
2. 在 `crates/douyin/src/process.rs::process_one`：在 `download_one` 与 `asr.transcribe_path`
   之间新增**可选前置清洗**——`job.clean_audio` 为真时调 `AudioCleanClient` 以
   `CleanOpts{ separate:true, denoise:true, pause:Off, sr:16000, level:Gentle, .. }` 清洗，
   落盘 `<aweme_id>.clean.wav` 后把它作为 `transcribe_path` 的输入；否则用原 mp4。
3. douyin `Job` 新增字段：`clean_audio: bool`、`clean_base_url: String`（默认 `http://127.0.0.1:8097`）；
   CLI 加 `--clean-audio` / `--clean-base-url` 参数。
4. toolkit-server audio 路由新增 `POST /api/web/audio/clean`：读 env **`CLEAN_BASE_URL`**
   → 用**该值**构造 `AudioCleanClient` 并转发（**不得**硬编码 `127.0.0.1:8097`）；
   **env 未配置则直接返回 503**（与现有 `TTS_BASE_URL` 未配返回 503 的约定一致）；
   env 已配但上游不可达返回 502。
5. toolkit `CLAUDE.md` 的「GB10 服务清单」补一行 `:8097 ← 音频清洗（streaming-speech 仓维护）`。

> zero-desktop 的 `speech_clean_recording` Tauri command 见 [Plan 4](audio-cleanup-plan-4.md)，
> 本 Plan 不实现。

### `CLEAN_BASE_URL` 行为（**消除「有默认又503」矛盾**）

| 层 | 默认 | 缺失/不可达行为 |
|---|---|---|
| `audio-clean-client` 库 | `http://127.0.0.1:8097` | 库层有默认（便利） |
| toolkit-server env `CLEAN_BASE_URL` | **无默认** | 未配置 → `/api/web/audio/clean` 返回 **503**；配置了但上游不可达 → **502** |

> 两层不矛盾：库默认是 Rust API 便利值；toolkit-server 的 env 故意无默认，缺失即 503，
> 与 `TTS_BASE_URL` 完全对齐。

## 行为规则

| 输入 | 期望结果 |
|---|---|
| douyin job `clean_audio=true`，视频带 BGM | 先 `/clean separate=1 pause=off sr=16000` 去乐 → 干净 wav 喂 `/transcribe`，识别率提升 |
| douyin job `clean_audio=false` | 维持现状：mp4 直接喂 `transcribe_path`，行为不变 |
| toolkit-server 收到 `/api/web/audio/clean` 且 `CLEAN_BASE_URL` 未配 | `503` |
| `CLEAN_BASE_URL` 已配但 `:8097` 不可达 | `502` |
| 调用方误把 `clean_base_url` 设成 `.../clean` | 客户端 `with_client` 内 trim 掉 `/clean` 后只拼一次，不会变成 `/clean/clean`（单测覆盖） |

## 禁止事项

- 不要改本仓（streaming-speech）任何文件。
- 不要改 `asr-client` 现有公开接口。
- 不要让 `audio-clean-client` 自动重试（与 asr-client 一致，重试由上层决策）。
- 不要给 toolkit-server 的 `CLEAN_BASE_URL` 设默认值（必须缺失即 503）。
- douyin 清洗路径不要用 `pause=drop`（删段破坏与画面/字幕时间对齐，固定 `off`）。
- 不要新增未经用户同意的依赖（reqwest/serde/anyhow 已在 workspace）。

## 测试 / 验证要求

- `audio-clean-client` 单测：`clean_base_url` 末端拼 `/clean` 正确；响应头 → `CleanedAudio`
  字段解析；HTTP 错误分类。
- douyin 单测：`clean_audio=false` 时不调清洗（mock 断言零调用）；`true` 时落盘 `.clean.wav`
  并以之为 transcribe 输入。
- toolkit-server 路由测试：`CLEAN_BASE_URL` 未配 → 503；上游 mock 不可达 → 502。
- 修复流程（toolkit 仓根）：`cargo clippy --workspace -- -D warnings` / `cargo fmt --check --all`
  / `cargo test --workspace` 三项全过。

## 完成条件

- [ ] `crates/audio-clean-client/` 实现 + 单测通过，`clean_base_url` 内部拼 `/clean`
- [ ] douyin `process_one` 接入可选前置清洗；`Job` 加 `clean_audio`/`clean_base_url`；CLI 加参数
- [ ] toolkit-server `/api/web/audio/clean` 代理就绪，`CLEAN_BASE_URL` 缺失→503、不可达→502
- [ ] toolkit `CLAUDE.md` 服务清单补登 `:8097`
- [ ] toolkit 仓修复流程三项全过

> zero-desktop 入口不在本 Plan 完成条件内（见 Plan 4）。
