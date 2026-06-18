# Plan: orchestrator 迁入 toolkit 仓(统一部署面板/运维)

> 日期 2026-06-18。动机:把 orchestrator 纳入 toolkit 的 **G10 部署面板**统一管理
> (与 `toolkit-server` 一样的 systemd 二进制 + 自更新 + 端口/env 注入),不再走本仓
> `release-server.ps1` 的 Docker compose 路径。
>
> 关联修复:本次 [docs/todo-2026-06-18-optimize-hang-no-trace.md](todo-2026-06-18-optimize-hang-no-trace.md)
> 的 LLM 超时/兜底已落地在 `server/orchestrator`,迁移应在该修复之后基线上进行。

## 进度(2026-06-18 更新)

代码迁移已落地并通过编译/测试,**剩运行时切换需在 GB10 现场执行**:

- [x] **Step 1** asr 发布 `127.0.0.1:9100:9100`(`server/compose.yaml`)。
- [x] **Step 2(已重新决策:不升 axum)** —— orchestrator 保持 axum 0.7,作为独立
  binary 与 toolkit 的 axum 0.8 在同 workspace 共存(实测 `cargo check` 通过),
  避开 0.8 的 WS Message 破坏性变更。原计划的升级取消。
- [x] **Step 3** orchestrator `main` 重构为 clap CLI(`serve`/`install`/`update`)+ watchdog,
  env 默认改回环(`ASR_WS=ws://127.0.0.1:9100` 等),workspace 默认 `~/.config/orchestrator`。
- [x] **Step 4** 移入 `toolkit/crates/orchestrator`,加入 workspace members;依赖对齐
  (custom-utils→workspace 0.16.0 带 `updater`;rusqlite 保留 0.38)。
  **实测:rusqlite 0.31(toolkit)与 0.38(orchestrator)在同 workspace 共存无冲突**
  ——两者是独立 binary、`libsqlite3-sys` 子树不相交;`toolkit-server`/`toolkit-core`/`rag`
  均仍正常编译。故无需对齐 rusqlite 版本。
- [x] **Step 5** `deploy-g10.ps1`:`$Bins` 加 orchestrator;新增 `$DaemonBins`,install/重启
  分支泛化为按 `$Service` 通用处理(toolkit-server 与 orchestrator install CLI 一致)。
- [ ] **Step 6** SQLite 数据迁移(GB10 现场,见下)。
- [ ] **Step 7** 切流量 + 退役旧路径(GB10 现场 + 文档更新)。

LLM 超时/兜底修复(todo-2026-06-18)已先在原仓 `server/orchestrator` 落地,再随文件
拷入 toolkit,故迁移版**已含**该修复。

> 迁移期 `server/orchestrator` 暂不删除(双存),待 Step 7 现场验收通过后再退役 +
> 留 MOVED.md(参照 `server/asr-server/MOVED.md`)。

## 0. 决策结论(先读)

- **orchestrator 搬,asr 不搬。** asr 是 Python + FunASR + GPU 容器,变不成裸二进制,
  继续留在本仓 Docker。orchestrator 是自包含 Rust(管理台 HTML 内联、SQLite 唯一外部
  状态、无磁盘静态资源),可干净交叉编译成单个 aarch64 二进制,契合 toolkit 模型。
- **唯一真实耦合改动**:orchestrator↔asr 现在是 compose 内网 WS(`ws://asr:9100`,asr
  只 `expose` 不发布)。orchestrator 变宿主进程后改走回环连 asr → **asr 需发布
  `127.0.0.1:9100:9100`**(与它已有的 `127.0.0.1:9101:9101` 同款)。
- **反向红利**:vLLM / trace 接线变简单——宿主进程直连 `:8085` / `:9100`,不再需要
  `host.docker.internal` + `extra_hosts`。trace 依赖两仓已同源(`custom-utils 0.16`)。

### 迁移后格局
```
toolkit 部署面板 ── orchestrator(systemd 二进制,:8090,~/.local/bin)
本仓 streaming-speech ── asr(唯一 GPU 容器,Docker,:9100 回环 + :9101)
                              ↑ orchestrator 经 127.0.0.1:9100 / :9101 连它
```

## 1. 前置:对照 toolkit-server 的"面板可管"形态

部署面板能管一个服务,要求该服务(参 `crates/toolkit-server`):
- 是 toolkit workspace 成员 crate,产出单 `[[bin]]`。
- 有 `prod` feature(`prod = ["custom-utils/prod"]`,日志落文件、stdout 保持干净 JSON)。
- 有 `clap` CLI + `install` 子命令(custom-utils updater 提供):写 systemd user unit,
  把 `--bind` 落成 `Environment=TOOLKIT_BIND=<bind>`,支持 `--workspace` / `--env KEY=VAL`。
- 被 `deploy-g10.ps1` 的 `$Bins` 收录;面板把 registry 主端口拼成 `0.0.0.0:<port>` 经
  `-Bind` 传入,额外 env 经 `-Env` 传入。

orchestrator 现状差距:**纯 env-var 配置,无 clap、无 install 子命令**
(`main.rs:47-51` 直接读 `ORCH_BIND`/`ASR_WS`/`VLLM_BASE`/`DATA_DIR`/`TRACE_HUB_ENDPOINT`)。

## 2. 步骤

### Step 1 — asr 暴露 9100 到回环(本仓,先做、可独立验证)
- `server/compose.yaml` 的 `asr` 服务:`ports` 增 `"127.0.0.1:9100:9100"`(保留 `expose`)。
- 部署 asr,验证宿主 `curl`/ws 能连 `127.0.0.1:9100`。
- 这步与 orchestrator 是否搬无关,先落地不破坏现状(orchestrator 容器仍走内网名 `asr`)。

### Step 2 — axum 版本(已决策:**不升级**)
- 原计划把 orchestrator 升到 toolkit workspace 的 axum 0.8。**实施时改为保留 0.7**:
  orchestrator 是独立 binary,在 Cargo.toml 显式写 `axum = "0.7"`(不用 `workspace = true`),
  与 toolkit 的 0.8 在同 workspace 并存(不同 major,Cargo 允许;两者属不同 binary,互不影响)。
- 理由:axum 0.8 改了 WS `Message` 类型(`Text(Utf8Bytes)`/`Binary(Bytes)`),orchestrator
  的 PCM 热路径直接用这些变体,升级风险/收益不划算;保留 0.7 零改动且 `cargo check` 已验证通过。
- 代价:workspace 多编一份 axum 0.7。可接受。

### Step 3 — orchestrator 接 CLI + install 子命令(本仓或迁移时)
- 引入 `clap`(features `env`),把 `main()` 改成:默认 `serve`(读 env/参数起服务),
  新增 `install`(custom-utils updater:写 unit、`--bind`→`Environment=ORCH_BIND`、
  `--workspace`、`--env`)。参照 `crates/toolkit-server/src/main.rs` 的 install 分支。
- 配置项对齐 env 名:沿用 `ORCH_BIND` 作为 bind(面板 `-Bind` 注入),
  `ASR_WS`/`ASR_EMBED` 默认改为 `ws://127.0.0.1:9100` / `http://127.0.0.1:9101/embed`,
  `VLLM_BASE` 默认 `http://127.0.0.1:8085/v1`,`TRACE_HUB_ENDPOINT` 默认 `http://127.0.0.1:9100/v1/spans`。
- 加 `prod` feature。

### Step 4 — 移入 toolkit workspace
- 把 `server/orchestrator/{src,Cargo.toml}` 拷到 `D:\git\toolkit\crates\orchestrator`。
- Cargo.toml 改 workspace 风格:`version.workspace = true`、依赖尽量 `{ workspace = true }`
  (`axum`/`tokio`/`reqwest`/`rusqlite`/`serde`/`custom-utils`/`anyhow`/`chrono` 都已在
  toolkit `[workspace.dependencies]`),独有依赖(如 `tokio-tungstenite`、`futures-util`)
  本 crate 内声明。
- toolkit 根 `Cargo.toml` 的 `members` 加 `"crates/orchestrator"`。
- `cargo check`(toolkit 工作区) + 交叉编译镜像内 `aarch64` build 验证。

### Step 5 — 接 deploy-g10 / 部署面板
- `deploy-g10.ps1` 的 `$Bins` 加 `@{ Crate = "orchestrator"; Bin = "orchestrator" }`。
- 脚本里"仅 toolkit-server 是 daemon、做 install"的分支泛化:让 orchestrator 也走
  install(它现在也是 daemon)。建议把 Service→install 命令做成可配置,而非硬编死
  `toolkit-server`。
- 在 G10 部署面板登记 orchestrator 服务:registry 主端口 8090,需要的 env(ASR_WS 等若
  非默认)。

### Step 6 — SQLite 数据迁移(GB10 现场,具体命令)
现状:docker named volume(compose 项目名前缀,通常 `server_orch-data`)挂 `/data`,
内含 `segments`/`sessions`/`speakers`/`config` 表。**config 表别丢**:
`asr.model`/`asr.secondary_model`/`vllm.model`/`llm.*_prompt` 等运行时配置 + 声纹都在库里。

```bash
# GB10 上,先确认卷名
docker volume ls | grep orch-data
# 从旧卷导出 app.db 到新 workspace
mkdir -p ~/.config/orchestrator
docker run --rm -v server_orch-data:/data -v ~/.config/orchestrator:/out \
  alpine sh -c 'cp -v /data/app.db /out/app.db'
# 校验 config 行数非空(迁移前后应一致)
sqlite3 ~/.config/orchestrator/app.db 'select count(*) from config;'
```

### Step 7 — 切流量 + 退役旧路径(GB10 现场,顺序敏感)
**顺序**:先让 asr 暴露 9100 回环 → 迁数据 → 起新 orchestrator → 停旧容器。

```powershell
# 1) 本仓:部署带 9100 回环的 asr(Step 1 已改 compose)
.\scripts\release-server.ps1 -Service asr
```
```bash
# 2) GB10:做 Step 6 数据迁移(见上)
```
```powershell
# 3) toolkit:部署 + 安装 orchestrator(systemd 二进制)
pwsh ./deploy-g10.ps1 -Service orchestrator -Bind 0.0.0.0:8090
```
```bash
# 4) GB10:首次需 enable + 开机自起(install 若未自动 enable)
systemctl --user enable --now orchestrator
loginctl enable-linger fengqi   # 确保登出/重启后用户服务仍运行
# 5) 停旧 docker orchestrator,asr 保留
cd ~/server && docker compose stop orchestrator && docker compose rm -f orchestrator
```
- 客户端 `ws://192.168.0.68:8090/stream` 不变(端口一致),零改动。
- 验证全链路:客户端连上 → segment/optimized/translated → 管理台 `:8090/` 可开 →
  trace-hub 有记录 → 声纹注册(`/api/speakers/enroll` → asr `:9101`)。
- 收尾:本仓 `scripts/release-server.ps1` 的 `orchestrator`/`both` 分支退役(只剩 asr);
  `server/compose.yaml` 删 `orchestrator` 服务;`server/orchestrator/` 留 MOVED.md 退役;
  更新 CLAUDE.md / docs/DEPLOYMENT.md 指向 toolkit 的 deploy-g10 面板。

## 3. 风险与注意

| 风险 | 说明 / 缓解 |
|---|---|
| axum 版本 | 已决策不升级:orchestrator 保留 0.7(独立 binary,与 toolkit 0.8 共存),零改动、已编译通过。 |
| rusqlite 版本冲突 | 已验证 0.38(orchestrator)与 0.31(toolkit)同 workspace 共存,无需对齐。 |
| asr 9100 暴露面 | 仅 `127.0.0.1`,不上 LAN;与 9101 同策略。 |
| config 表丢失 | Step 6 显式校验 `config` 行数;迁移前后 diff。 |
| 面板脚本只认 toolkit-server | Step 5 需泛化 daemon install 分支(否则 orchestrator 不会被 install/重启)。 |
| asr 与 orchestrator 分仓后联调更碎 | 接受:asr 接口稳定(内部 WS + `/transcribe`/`/embed`),两者经回环松耦合。 |
| 客户端默认 URL | 端口仍 8090,默认 `ws://192.168.0.68:8090/stream` 不变,客户端零改动。 |

## 4. 验收

- 部署面板能 install / 重启 / 改端口 orchestrator(与 toolkit-server 体验一致)。
- 客户端全链路正常:实时 segment + 异步 optimized/translated + 次模型对比 + 声纹。
- 管理台 `:8090/` 正常;trace-hub 有 orchestrator 链路记录(含本次超时修复的失败分支)。
- 旧 docker orchestrator 容器与 `release-server.ps1` orchestrator 分支已退役,文档已更新。

## 5. 不做 / 边界

- **asr 不迁**(GPU Python 容器)。
- 不在本计划内动 vLLM(主机进程,带外维护)与 TTS bake-off。
- 不改 WS 协议、不改客户端(端口/路径不变)。
