# Rust 编排层 — 多阶段构建,产物极小静态化镜像(arm64 GB10)
FROM rust:1-bookworm AS build
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
