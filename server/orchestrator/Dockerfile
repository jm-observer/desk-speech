# Rust 编排层 — 多阶段构建,产物极小静态化镜像(arm64 GB10)
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
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/orchestrator /usr/local/bin/orchestrator
ENV ORCH_BIND=0.0.0.0:8090
EXPOSE 8090
CMD ["/usr/local/bin/orchestrator"]
