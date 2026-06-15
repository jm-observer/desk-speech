# Plan 4: zero-desktop 桌面端清洗入口

## 前置依赖

Plan 3（toolkit 联动）—— 复用其 toolkit-server `POST /api/web/audio/clean` 代理路由；
zero-desktop 不直连 GB10 :8097。

> **跨仓说明**：改动全部在 **toolkit 仓**的 `crates/zero-desktop/`。zero-desktop 跑在本地
> Windows，按现有「桌面端经 toolkit-server 代理、不直连 GB10」先例（TTS 走
> `/api/web/audio/tts` 即此模式）接入。

## 任务目标

zero-desktop Speech 模块新增一个 Tauri command，把本地麦克风录音经 toolkit-server 代理清洗后
再上传/听写，让桌面端用户拿到去噪后的录音。

## 执行范围

- **必须新增/修改（toolkit 仓 `crates/zero-desktop/`）**：Speech 模块新增 `speech_clean_recording`
  command + 前端调用入口。
- **禁止修改**：`audio-clean-client`、toolkit-server 代理路由（属 Plan 3，本 Plan 只消费）；
  本仓（streaming-speech）任何文件；asr-client。

## Agent 执行步骤

1. 在 zero-desktop Speech 模块新增 Tauri command `speech_clean_recording`：入参为本地录音文件
   路径 + 清洗选项（至少 `denoise`、`pause`），调用 toolkit-server 的 `POST /api/web/audio/clean`
   （base 取桌面端已配置的 toolkit-server 地址，与 Speech 现有上传命令同源），把返回的干净音频
   **并列**落盘为 cleaned variant（如 `<原名>.cleaned.wav` + 录音库新增一条关联记录），
   **不得覆盖原录音文件**。
2. 前端 Speech 界面加一个「清洗录音」入口，触发该 command，完成后**并列展示**原录音与
   cleaned 版（不替换、不删除原录音）。
3. 错误透传：代理返回 503（`CLEAN_BASE_URL` 未配）/ 502（上游不可达）时，前端给出可读提示，
   不静默吞错。

## 行为规则

| 输入 | 期望结果 |
|---|---|
| 桌面端对一段本地录音点「清洗录音」 | 经 `/api/web/audio/clean` 清洗 → cleaned variant **并列**落盘，原录音保留不动 |
| toolkit-server 返回 503 | 前端提示「清洗服务未配置」，不崩溃 |
| toolkit-server 返回 502 | 前端提示「清洗服务不可达」，不崩溃 |

## 禁止事项

- 不要让 zero-desktop 直连 GB10 `:8097`（必须走 toolkit-server :8788 代理）。
- 不要在本 Plan 改代理路由或 client crate（属 Plan 3）。
- 不要静默吞 503/502。
- **不要覆盖或删除原录音**——cleaned 版必须并列保存。

## 测试 / 验证要求

- command 单测/集成：mock 代理返回 200 → cleaned variant 并列落盘且**原文件仍在**（断言原录音
  字节未变）；返回 503/502 → 命令返回可读错误。
- 修复流程（toolkit 仓根）：`cargo clippy --workspace -- -D warnings` / `cargo fmt --check --all`
  / `cargo test --workspace` 三项全过。

## 完成条件

- [ ] `speech_clean_recording` command 实现，经代理清洗并**并列**落盘 cleaned variant（原录音不动）
- [ ] 前端 Speech 界面有清洗入口，原录音与 cleaned 版并列展示
- [ ] 503/502 错误前端可读、不崩溃
- [ ] toolkit 仓修复流程三项全过
