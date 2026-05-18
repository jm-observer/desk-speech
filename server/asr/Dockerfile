# FunASR ASR service — arm64 / Blackwell(sm_121) / CUDA 13
# 已在 GB10 验证可跑通(Paraformer-large + FSMN-VAD + CT-punc, GPU)。
FROM nvidia/cuda:13.0.0-cudnn-runtime-ubuntu24.04
ENV DEBIAN_FRONTEND=noninteractive PIP_NO_CACHE_DIR=1

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 python3-pip python3-venv libsndfile1 ffmpeg ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# torch/triton/torchaudio: arm64 cu130 nightly。
# 慢网建议预下大 wheel 放 ./wheels/ 后用 --find-links;否则直接走索引(已加大超时/重试)。
COPY wheels/ /wheels/
RUN pip3 install --break-system-packages --pre --default-timeout=2000 --retries 10 \
    --index-url https://download.pytorch.org/whl/nightly/cu130 \
    --find-links /wheels \
    torch triton torchaudio

RUN pip3 install --break-system-packages --default-timeout=600 --retries 10 \
    funasr modelscope soundfile numpy "websockets>=12"

WORKDIR /app
COPY app.py /app/app.py

# 模型目录由 compose 通过卷挂载 + 环境变量指定(换模型只改 env/挂载)
ENV ASR_PARAFORMER_DIR=/models/speech_paraformer-large_asr_nat-zh-cn-16k-common-vocab8404-pytorch \
    ASR_VAD_DIR=/models/speech_fsmn_vad_zh-cn-16k-common-pytorch \
    ASR_PUNC_DIR=/models/punc_ct-transformer_zh-cn-common-vocab272727-pytorch \
    ASR_DEVICE=cuda \
    ASR_PORT=9100

EXPOSE 9100
CMD ["python3", "/app/app.py"]
