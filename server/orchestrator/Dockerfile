# syntax=docker/dockerfile:1.7
# Rust 编排层 — 多阶段 + BuildKit cache mounts。GB10 网络不稳,缓存掉 cargo
# registry / git / target,代码改动后增量 build 约 30s-2min(vs 冷构 15min+)。
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
WORKDIR /src

# ── Layer A: 依赖层。仅当 Cargo.toml/lock 变动时重跑。
# 用 dummy main.rs 触发依赖编译,真实源码不参与,这样改 src/ 不会让 deps 失效。
COPY Cargo.toml Cargo.lock ./
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
