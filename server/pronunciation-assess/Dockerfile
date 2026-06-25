# 英语发音评测(GOP)服务。薄层,基于 GB10 已验证镜像 funasr-asr:arm64
# (含 torch 2.13.dev+cu130 / arm64 / sm_121 / ffmpeg / torchaudio)。这里只补评测所需依赖。
#
# 关键:装 transformers/g2p_en 时绝不能让 pip 把 torch 拉成 PyPI 正式版——
# 下方构建期断言守住这条线(torch 必须仍是 2.13 + cu130)。
FROM funasr-asr:arm64

# g2p_en 依赖 nltk(需 cmudict / averaged_perceptron_tagger 语料)。pip 走 Aliyun 镜像
# (pypi.org/清华在 GB10 SSL 抖动);--no-cache-dir 避免坏缓存。不重装 torch/torchaudio。
RUN pip3 install --break-system-packages --no-cache-dir \
    --default-timeout=600 --retries 10 \
    -i https://mirrors.aliyun.com/pypi/simple/ \
    "aiohttp>=3.9" "transformers>=4.40" g2p_en nltk

# 预下 nltk 语料进镜像(GB10 运行期外网不稳;g2p_en 首次 import 会取 tagger + cmudict,缺则崩)。
# nltk>=3.9 把 tagger 改名为 averaged_perceptron_tagger_eng;旧名一并下,兼容不同 g2p_en/nltk 组合。
# 下载源走 nltk 默认,构建机需可达;失败则在运行期挂卷补(见 README「离线」)。
RUN python3 -m nltk.downloader -d /usr/share/nltk_data \
        averaged_perceptron_tagger averaged_perceptron_tagger_eng cmudict || true
ENV NLTK_DATA=/usr/share/nltk_data

# 构建期断言:torch 必须仍是 2.13 + cu130,否则依赖偷偷升级了 torch,立即让构建失败。
RUN python3 -c "import torch; v=torch.__version__; assert v.startswith('2.13') and 'cu130' in v, 'torch drifted: '+v; print('torch OK', v)" \
 && python3 -c "import torchaudio; print('torchaudio', torchaudio.__version__)"

WORKDIR /app
COPY gop.py /app/gop.py
COPY app.py /app/app.py

# GOP_MODEL_ID:wav2vec2 CTC 音素模型(输出 ARPAbet);换模型只改这里(见 README「模型」)。
# 权重经宿主缓存卷持久化(compose 挂 /root/.cache/huggingface),不进镜像。
ENV GOP_PORT=8098 \
    GOP_DEVICE=cuda \
    GOP_MODEL_ID=slplab/wav2vec2-large-robust-L2-english-phoneme-recognition \
    HF_ENDPOINT=https://hf-mirror.com

EXPOSE 8098
CMD ["python3", "/app/app.py"]
