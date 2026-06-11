# Rust 编排层 — 多阶段 + BuildKit cache mounts。GB10 网络不稳,缓存掉 cargo
# registry / git / target,代码改动后增量 build 约 30s-2min(vs 冷构 15min+)。
# 不写 `# syntax=docker/dockerfile:x` 指令:它每次构建都要联网解析 Docker Hub
# manifest,GB10 经常 TLS 超时直接挡住构建;本文件只用 cache mounts,dockerd
# 内置前端即支持,无需外部 frontend 镜像。
FROM rust:1-bookworm AS build
# GB10 has poor connectivity to crates.io; use a China mirror (sparse).
RUN mkdir -p $CARGO_HOME && printf '%s\n' \
    '[source.crates-io]' \
    "replace-with = 'rsproxy-sparse'" \
    '[source.rsproxy-sparse]' \
    'registry = "sparse+https://rsproxy.cn/index/"' \
    '[net]' \
    'retry = 5' \
    '[http]' \
    'timeout = 120' \
    > $CARGO_HOME/config.toml
# reqwest 0.13 的 rustls 后端拉 aws-lc-rs，其 aws-lc-sys 构建期需 cmake（0.12 的 ring 不需要）。
RUN apt-get update && apt-get install -y --no-install-recommends cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /src

# ── Layer A: 依赖层。仅当 Cargo.toml 变动时重跑。
# 用 dummy main.rs 触发依赖编译,真实源码不参与,这样改 src/ 不会让 deps 失效。
# 注:orchestrator 已并入仓库根 workspace,Cargo.lock 移到仓库根;本镜像仍以
# orchestrator 子目录为上下文独立编译(容器内无父 workspace),由 cargo 现场生成 lock
# ——与 asr-server 镜像一致。
COPY Cargo.toml ./
RUN mkdir -p src && echo 'fn main(){}' > src/main.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release && rm -rf src

# ── Layer B: 真实源码。改 src/ 只重跑这层,deps 命中 cache mount。
# 注意:cache mount 在 RUN 退出后会卸载,产物在镜像层里看不见,所以同一个 RUN
# 内 cp 到非缓存路径。
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    touch src/main.rs && \
    cargo build --release && \
    cp /src/target/release/orchestrator /usr/local/bin/orchestrator

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/orchestrator /usr/local/bin/orchestrator
ENV ORCH_BIND=0.0.0.0:8090
EXPOSE 8090
CMD ["/usr/local/bin/orchestrator"]
