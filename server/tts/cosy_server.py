"""CosyVoice 2 zero-shot / instruct synth service (bake-off harness).

Standalone — not wired into the production asr/orchestrator. Exists so we can
feed a reference clip + text and hear cloned + emotion-controlled output.

Two GB10-specific shims, both proven necessary in the tts-scratch session:
  * torchaudio 2.11-dev (paired with the cu130 GB10 torch) dropped the
    soundfile backend and forces torchcodec, which has no matching arm64
    wheel. CosyVoice's load_wav() calls torchaudio.load/save internally, so
    we monkeypatch them onto soundfile BEFORE importing cosyvoice.
  * Current CosyVoice main takes prompt_wav as a FILE PATH (it loads inside
    the frontend), not the old pre-loaded tensor — so we spool uploads to a
    temp wav and hand over the path.

Endpoints:
  GET  /health
  POST /tts/zero_shot   form: tts_text, prompt_text ; file: prompt_wav -> audio/wav
  POST /tts/instruct    form: tts_text, instruct     ; file: prompt_wav -> audio/wav
      instruct example: 用开心的语气说 / 用四川话说 / 慢慢地、温柔地说
"""
import io
import os
import sys
import tempfile

CV_DIR = os.environ.get("COSYVOICE_DIR", "/app/CosyVoice")
sys.path.insert(0, CV_DIR)
sys.path.insert(0, os.path.join(CV_DIR, "third_party", "Matcha-TTS"))

import soundfile as sf
import torch
import torchaudio


def _ta_load(filepath, *a, **k):
    data, sr = sf.read(filepath, dtype="float32", always_2d=True)  # (T, C)
    return torch.from_numpy(data.T).contiguous(), sr  # (C, T), sr


def _ta_save(filepath, src, sample_rate, *a, **k):
    x = src.detach().cpu().numpy()
    sf.write(filepath, x.T if x.ndim == 2 else x, sample_rate)


torchaudio.load = _ta_load
torchaudio.save = _ta_save

from fastapi import FastAPI, File, Form, UploadFile
from fastapi.responses import Response

from cosyvoice.cli.cosyvoice import CosyVoice2

MODEL_DIR = os.environ.get("COSYVOICE_MODEL", "/models/CosyVoice2-0.5B")
FP16 = os.environ.get("COSYVOICE_FP16", "0") == "1"

app = FastAPI()
_model = None


def model() -> CosyVoice2:
    global _model
    if _model is None:
        _model = CosyVoice2(MODEL_DIR, load_jit=False, load_trt=False, fp16=FP16)
    return _model


def _spool(raw: bytes) -> str:
    fd, path = tempfile.mkstemp(suffix=".wav")
    with os.fdopen(fd, "wb") as fh:
        fh.write(raw)
    return path


def _wav_bytes(chunks) -> bytes:
    speech = torch.cat([c["tts_speech"] for c in chunks], dim=1)
    arr = speech.squeeze(0).cpu().numpy()
    buf = io.BytesIO()
    sf.write(buf, arr, model().sample_rate, format="WAV")
    return buf.getvalue()


@app.get("/health")
def health():
    return {"ok": True, "model_loaded": _model is not None, "fp16": FP16}


@app.post("/tts/zero_shot")
async def zero_shot(
    tts_text: str = Form(...),
    prompt_text: str = Form(...),
    prompt_wav: UploadFile = File(...),
):
    path = _spool(await prompt_wav.read())
    try:
        chunks = list(
            model().inference_zero_shot(tts_text, prompt_text, path, stream=False)
        )
        return Response(content=_wav_bytes(chunks), media_type="audio/wav")
    finally:
        os.unlink(path)


@app.post("/tts/instruct")
async def instruct(
    tts_text: str = Form(...),
    instruct: str = Form(...),
    prompt_wav: UploadFile = File(...),
):
    path = _spool(await prompt_wav.read())
    try:
        chunks = list(
            model().inference_instruct2(tts_text, instruct, path, stream=False)
        )
        return Response(content=_wav_bytes(chunks), media_type="audio/wav")
    finally:
        os.unlink(path)


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=int(os.environ.get("TTS_PORT", "8095")))
