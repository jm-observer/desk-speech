# server/ — GB10 服务端(P0 脚手架)

客户端 ↔ 编排层 ↔ ASR/vLLM。详见 `../docs/redesign-architecture-overview.md`、
`../docs/p0-plan.md`、`../docs/protocol-draft.md`。

```
server/
  asr/            FunASR ASR 服务(Python sidecar,模型走 env,可换模型)
  orchestrator/   Rust 编排层(对客户端的 WebSocket;串 ASR + vLLM;发协议事件)
  compose.yaml    docker compose 编排(GB10 上跑)
```

## 设计不变量(换模型不伤架构的"缝")

- ASR 服务契约固定:**音频(16k PCM)→ 段(文本+时间)**,与具体模型无关
- 模型由 **环境变量** 选(`ASR_*_DIR`),不写死;换 Paraformer↔SenseVoice↔faster-whisper 只改 env/挂载
- 保留 `hello.language` 路由位:后续按语言在 asr 内部切模型,协议/编排/客户端无感

## 在 GB10 上跑(P0)

模型与镜像构建产物当前在 `~/funasr-prep/`(已验证)。compose 通过卷挂载模型目录,
镜像基于已验证的 arm64/Blackwell/CUDA13 方案。具体启动步骤随实现补全。

> 现状:**脚手架**。结构与协议形状已就位,标注 `TODO` 处为后续逐步硬化(联调时迭代)。
