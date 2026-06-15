# 音频清洗服务（Plan 2）。薄层，基于 GB10 已验证镜像 funasr-asr:arm64
# （含 torch 2.13.dev+cu130 / arm64 / sm_121 / ffmpeg）。这里只补清洗所需依赖。
#
# 关键：装 demucs/deepfilternet 时绝不能让 pip 把 torch 拉成 PyPI 正式版——
# 下方构建期断言守住这条线（torch 必须仍是 2.13 + cu130）。
FROM funasr-asr:arm64

# DeepFilterLib（deepfilternet 的 Rust DSP 后端）arm64 无预编译 wheel，需现编。基镜像无
# curl/gcc/rust —— 补齐：apt 装 curl+build-essential，rustup 装 stable，cargo 源走 rsproxy.cn
# （GB10 直连 crates.io / 官方 rust 不稳；与本仓其他 server Dockerfile 同惯例）。
# git：DeepFilterNet 的 init_logger 会 `git` 取自身 commit（缺 git 则 FileNotFoundError 直接崩）。
RUN apt-get update && apt-get install -y --no-install-recommends curl build-essential git \
 && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_DIST_SERVER=https://rsproxy.cn \
    RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup \
    PATH=/root/.cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://rsproxy.cn/rustup-init.sh | sh -s -- -y --default-toolchain stable --profile minimal \
 && printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n[net]\ngit-fetch-with-cli = true\n' > /root/.cargo/config.toml

# pip 走 Aliyun 镜像（pypi.org/清华在 GB10 SSL 抖动）；--no-cache-dir 避免坏缓存。
# 不重装 torch/torchaudio：复用基础镜像的 dev 版。DeepFilterLib 会用上面的 Rust+rsproxy 现编。
RUN pip3 install --break-system-packages --no-cache-dir \
    --default-timeout=600 --retries 10 \
    -i https://mirrors.aliyun.com/pypi/simple/ \
    "aiohttp>=3.9" demucs deepfilternet pyloudnorm silero-vad

# 构建期断言：torch 必须仍是 2.13 + cu130，否则依赖偷偷升级了 torch，立即让构建失败。
RUN python3 -c "import torch; v=torch.__version__; assert v.startswith('2.13') and 'cu130' in v, 'torch drifted: '+v; print('torch OK', v)" \
 && python3 -c "import torchaudio; print('torchaudio', torchaudio.__version__)"

WORKDIR /app
COPY pipeline.py /app/pipeline.py
COPY app.py /app/app.py

# CLEAN_DEMUCS_DEVICE：Plan 1 spike 实测拍板 cuda（gpu_peak 0.91GB、~4× cpu）。
ENV CLEAN_PORT=8097 \
    CLEAN_DEMUCS_DEVICE=cuda

EXPOSE 8097
CMD ["python3", "/app/app.py"]
