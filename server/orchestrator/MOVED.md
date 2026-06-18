# orchestrator 已迁出本仓(2026-06)

`server/orchestrator/`(对桌面客户端的 WebSocket 终结 + SQLite + Web 管理台 + HTTP API)
已于 2026-06 从本仓迁出至 **toolkit 中台仓库**(`jm-observer/toolkit`,本地
`D:\git\toolkit`)的 **`crates/orchestrator`**,并改变部署形态。

## 为什么迁

统一运维:让 orchestrator 进 toolkit 的 **G10 部署面板**,与 `toolkit-server` 一样走
systemd 二进制 + 自更新 + 端口/env 注入,不再用本仓的 Docker compose + `release-server.ps1`。

## 现在的权威源 & 部署

- **代码**:`D:\git\toolkit\crates\orchestrator`(toolkit workspace 成员)。
- **部署**:在 toolkit 仓 `pwsh ./deploy-g10.ps1 -Service orchestrator -Bind 0.0.0.0:8090`
  (交叉编译 aarch64 → scp `~/.local/bin/orchestrator` → `orchestrator install` 写
  systemd user unit)。GB10 上是 `systemctl --user` 服务,workspace/DB 在
  `~/.config/orchestrator/app.db`。
- **形态变化**:容器 → 宿主进程。故连接地址改回环:
  - asr:本仓 `server/asr` 仍是 Docker,compose 把内部 WS 9100 发布到宿主
    `127.0.0.1:9110`(宿主 9100 被 trace-hub 占用),orchestrator 用
    `ASR_WS=ws://127.0.0.1:9110`;声纹 embed 走 `127.0.0.1:9101`。
  - vLLM:宿主进程 `127.0.0.1:12340`(DB `vllm.base` 已据此改写)。
  - trace-hub:宿主 `127.0.0.1:9100`(install 注入 `TRACE_HUB_ENDPOINT`)。

## 客户端

URL 不变:`ws://192.168.0.68:8090/stream`。桌面客户端零改动。

## 本仓侧已完成的退役

- 删除 `server/orchestrator/{src,Cargo.toml,Dockerfile,.dockerignore}`(仅留本文件)。
- 根 `Cargo.toml` 移除 workspace 成员(现仅 `src-tauri`)。
- `server/compose.yaml` 删 `orchestrator` 服务(仅剩 `asr`)。
- `scripts/release-server.ps1` 退 orchestrator/both 分支(现只管 `asr`)。

迁移全过程与现场切换记录见
[`docs/plan-2026-06-18-orchestrator-move-to-toolkit.md`](../../docs/plan-2026-06-18-orchestrator-move-to-toolkit.md)。
