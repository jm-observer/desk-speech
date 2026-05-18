# 桌面客户端改造方案(P0)

> 把现有 Tauri app 从"本地识别单体"改成"瘦客户端 + 协议"。规格/计划级,非代码。
> ⚠️ 依 P0 §10:**FunASR 闸门通过前不动客户端代码**;本文是闸门一过即可执行的就绪方案。

---

## 1. 目标

- 复用现有壳、UI、剪贴板、采麦;**去掉本地推理与模型/打包负担**
- 录音→把音频经协议推 GB10,识别/优化/翻译结果经协议回来,UI 几乎不改

---

## 2. 去掉(P0)

| 移除项 | 涉及 |
|---|---|
| 本地 sherpa-onnx 识别 | `sherpa-onnx` crate 依赖;`build_models`/recognizer/VAD 创建 |
| 模型注册/选择 | `model_registry.rs`;`asr_model/asr_provider/asr_language` 的"本地生效"逻辑(改为 `hello` 参数透传) |
| 本地 VAD | 决策 E:VAD 移服务端;客户端不再切段 |
| 声纹门控(客户端侧) | `speaker.rs` + 录音管线里的 gate(移服务端,P1) |
| GPU 构建接线 | `.cargo/config.toml` 的 `SHERPA_ONNX_LIB_DIR`、`sherpa-gpu/`、shared 特性 |
| 模型随包/打包 | `tauri.conf.json` 的 `bundle.resources:["assets"]`、NSIS 大文件问题 —— **彻底消失**(客户端无模型) |

→ 顺带根除之前 2.6GB 模型 / DLL / 安装包之苦。

## 3. 保留

- 采麦:cpal → 16k 重采样(产出 16k PCM 帧)——**保留并复用**
- 前端 UI:录音/分段展示/设置/历史视图——基本不动,改数据来源
- 剪贴板自动复制(收到 `optimized` 后本地写)——**保留(必须原生)**
- 设置:麦克风设备、自动复制模式、语言/是否翻译(后两者改为 `hello` 参数)
- Tauri 命令面:`start_recording`/`stop_recording` 等对前端接口保持

## 4. 新增

- **协议客户端模块**(Rust):
  - 连接 WS → 发 `hello` → 把采麦 PCM 帧持续上行 → 收事件
  - 把协议事件映射成**前端已在消费的事件**(如现有 `segment-update`/`correction-applied` 等),使前端改动最小
  - 处理 `stop`/`done`/`error`/断线

## 5. 数据流(P0)

```
点击录音 → 建 WS + hello(language/want_optimize/want_translate)
  → cpal 采麦 → 16k PCM 帧 → WS 上行(持续)
  ← segment      → 前端显示该段
  ← optimized    → 就地更新 + 按设置写剪贴板
  ← translated   → 就地更新译文
停止 → 发 stop → 收 done → 关连接
```

## 6. 涉及文件(预估)

| 文件 | 改动 |
|---|---|
| `src-tauri/Cargo.toml` | 去 `sherpa-onnx`;加 ws 客户端依赖(tokio-tungstenite 等) |
| `src-tauri/src/lib.rs` | 删 `build_models`/recognizer/VAD/init 那套;AppState 瘦身;加 ws 会话生命周期 |
| `src-tauri/src/commands/recording.rs` | 采麦保留;把"本地 VAD+识别"换成"PCM 帧上行 + 事件回灌" |
| `src-tauri/src/model_registry.rs`、`speaker.rs` | P0 移除(speaker 逻辑后续在服务端) |
| `src-tauri/tauri.conf.json` | 去 `bundle.resources`/模型;客户端不再带模型 |
| `src-tauri/.cargo/config.toml` | 去 `SHERPA_ONNX_LIB_DIR`(GPU 链接不再需要) |
| `src/src/*`(前端) | 尽量不动:仅在事件来源/字段对齐处微调 |
| `settings.rs` | 区分"客户端本地设置"与"`hello` 透传参数";模型/provider 移服务端关注 |

## 7. 设置归属(P0)

- **客户端本地**:麦克风设备、自动复制模式、是否翻译、语言选择(后三者随 `hello` 发给服务端)
- **服务端**:模型/provider/识别引擎细节(P0 服务端固定 FunASR;管理台 P1 再做可视化切换)

## 8. 风险

- **跟手感**:服务端 VAD + 网络往返,需实测端点延迟是否可接受;不行则评估客户端轻量门限(决策 E 已留后路)
- **采集节奏/背压**:按真实采集速率推流,注意 WS 写阻塞处理
- **断线体验**:P0 简单——断即结束会话 + 提示;续传后置
- **前端耦合**:尽量让协议事件对齐现有前端事件,降低改动面

## 9. 验收(对齐 P0 §8)

- 录中文短句 → 经 GB10 返回文本,质量不逊现状
- 优化/翻译正常,优化文本自动复制行为对齐现状
- 局域网端到端延迟可接受;关闭/断线干净

## 10. 执行前置

- ✅ 协议草案(`protocol-draft.md`)评审定稿
- ⛔ **FunASR 闸门(P0 §10 step1)必须先通过** —— 否则服务端形态可能变(换 ASR),影响协议/客户端
- 闸门过 + 协议定 → 按本文 §6 动手
