# asr-server (arm64 / GB10) — CPU-only v1
#
# v1 范围:CPU SenseVoice int8。GPU/CUDA13 是后续工作(需手 build sherpa-onnx
# native libs for arm64+CUDA13,prebuilt 只到 CUDA 12.x;memory project_gpu_build.md
# 记录了 Windows 端 GPU 设置经验,Linux arm64 类似但 toolchain 不同)。
#
# 模型(宿主):放到 ~/asr-server-models/sherpa-sense-voice/,需含 model.int8.onnx
# 和 tokens.txt。GB10 从 hf-mirror 拉(实测可用,GitHub 直连超时):
#   cd ~/asr-server-models/sherpa-sense-voice && \
#   curl -L -O https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx && \
#   curl -L -O https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt

FROM rust:1-bookworm AS build

# Crates 国内镜像(GB10 网络,与 orchestrator 一致)
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
# sherpa-onnx 用 features=["shared"]:sherpa-onnx-sys/build.rs 会从其 release 下
# linux-aarch64 prebuilt(github)。若 GB10 拉不到,可在宿主预下载并通过
# `--build-arg SHERPA_PREBUILT_URL=file://...` 或挂载方式注入,然后改用
# SHERPA_ONNX_LIB_DIR 跳过 build.rs 的下载。当前先尝试 build.rs 默认行为。
RUN cargo build --release --bin asr-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libgomp1 && rm -rf /var/lib/apt/lists/*

# sherpa-onnx-sys 把 prebuilt arm64 libs 解到 /src/target/release/ 根目录(实测路径)。
# 三个运行期必备:libsherpa-onnx-c-api.so / libonnxruntime.so / libsherpa-onnx-cxx-api.so。
COPY --from=build /src/target/release/asr-server               /usr/local/bin/asr-server
COPY --from=build /src/target/release/libsherpa-onnx-c-api.so  /usr/local/lib/sherpa/
COPY --from=build /src/target/release/libonnxruntime.so        /usr/local/lib/sherpa/
COPY --from=build /src/target/release/libsherpa-onnx-cxx-api.so /usr/local/lib/sherpa/
ENV LD_LIBRARY_PATH=/usr/local/lib/sherpa

EXPOSE 8091
ENTRYPOINT ["/usr/local/bin/asr-server"]
CMD ["--bind", "0.0.0.0:8091", "--model", "sense-voice", "--model-dir", "/models/sherpa-sense-voice"]
