# FunASR ASR service — 薄层,基于 GB10 已验证镜像 funasr-asr:arm64
# 该基础镜像已含 torch/triton/torchaudio + funasr/modelscope/soundfile/numpy
# (arm64 / Blackwell sm_121 / CUDA13,闸门测试已跑通)。这里只补服务化所需。
#
# 注意:首次需先有 funasr-asr:arm64 基镜像(见 ~/funasr-prep 的构建);
# 换模型不改本文件——模型由 compose 的 env(ASR_*_DIR)+ 卷挂载决定。
FROM funasr-asr:arm64

RUN pip3 install --break-system-packages --default-timeout=600 --retries 10 \
    "websockets>=12" "aiohttp>=3.9"

WORKDIR /app
COPY app.py /app/app.py

ENV ASR_PARAFORMER_DIR=/models/speech_paraformer-large_asr_nat-zh-cn-16k-common-vocab8404-pytorch \
    ASR_VAD_DIR=/models/speech_fsmn_vad_zh-cn-16k-common-pytorch \
    ASR_PUNC_DIR=/models/punc_ct-transformer_zh-cn-common-vocab272727-pytorch \
    ASR_SPK_DIR=/models/speech_campplus_sv_zh_en_16k-common_advanced \
    ASR_DEVICE=cuda \
    ASR_PORT=9100 \
    ASR_HTTP_PORT=9101 \
    ORCH_BASE=http://orchestrator:8090

EXPOSE 9100 9101
CMD ["python3", "/app/app.py"]
