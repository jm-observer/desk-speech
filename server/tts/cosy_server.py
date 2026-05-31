"""CosyVoice 2 voice-cloning TTS service — standalone HTTP API for other
projects to call. Not wired into the production asr/orchestrator.

Two GB10-specific shims, both proven necessary in the tts-scratch session:
  * torchaudio 2.11-dev (paired with the cu130 GB10 torch) dropped the
    soundfile backend and forces torchcodec, which has no matching arm64
    wheel. CosyVoice's load_wav() calls torchaudio.load/save internally, so
    we monkeypatch them onto soundfile BEFORE importing cosyvoice.
  * Current CosyVoice main takes prompt_wav as a FILE PATH (it loads inside
    the frontend), not the old pre-loaded tensor — so we spool uploads to a
    temp wav and hand over the path.

Endpoints (full reference: server/tts/API.md):
  GET  /health
  GET  /voices                        # list available voice presets
  POST /tts                  json     # convenience wrapper, voice_id-based
  POST /tts/zero_shot        form     # raw zero-shot clone (upload your own ref)
  POST /tts/instruct         form     # voice clone + emotion/pace control
  POST /tts/cross_lingual    form     # for [laughter] tokens / cross-language
"""
import io
import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Optional

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

from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from fastapi.responses import Response
from pydantic import BaseModel

from cosyvoice.cli.cosyvoice import CosyVoice2

MODEL_DIR = os.environ.get("COSYVOICE_MODEL", "/models/CosyVoice2-0.5B")
FP16 = os.environ.get("COSYVOICE_FP16", "0") == "1"
VOICES_DIR = Path(os.environ.get("VOICES_DIR", "/voices"))

app = FastAPI(title="CosyVoice2 TTS", version="1.0")
_model = None
_manifest_cache: Optional[dict] = None


def manifest() -> dict:
    """Load voices.json on first access. Re-reads on every call to /voices so
    operators can hot-edit the file without restarting the container."""
    global _manifest_cache
    p = VOICES_DIR / "voices.json"
    if not p.exists():
        return {"prompt_text": "", "voices": []}
    return json.loads(p.read_text(encoding="utf-8"))


def lookup_voice(voice_id: str) -> tuple[str, str]:
    """Resolve voice_id → (wav_path, prompt_text). Per-voice
    prompt_text_override beats the manifest-level default. Raises 404 if the
    id or file is missing."""
    m = manifest()
    default_pt = m.get("prompt_text", "")
    for v in m.get("voices", []):
        if v["id"] == voice_id:
            wav = VOICES_DIR / v["file"]
            if not wav.exists():
                raise HTTPException(
                    500, f"voice '{voice_id}' wav missing on disk: {v['file']}")
            return str(wav), v.get("prompt_text_override", default_pt)
    raise HTTPException(404, f"unknown voice_id: {voice_id}")


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
    """Synthesize CosyVoice2 output chunks → WAV bytes (24 kHz mono 16-bit PCM)。

    历史：2026-05-31 曾改为 mp3 输出（推测 WeChat 不接受 wav）。后查官方
    npm 文档 README 明示 `voice_item` = "Voice (SILK encoded)"——微信客户端
    实际只识别 SILK 编码（encode_type=6），mp3 也不显示气泡。修复回到 SDK
    层：weixin-agent-sdk-rs 做 wav→silk 转码，本端恢复 wav 输出。
    详见 zero 仓 docs/adr/2026-05-31-english-coach-router-entry.md。
    """
    speech = torch.cat([c["tts_speech"] for c in chunks], dim=1)
    arr = speech.squeeze(0).cpu().numpy()
    buf = io.BytesIO()
    sf.write(buf, arr, model().sample_rate, format="WAV")
    return buf.getvalue()


@app.get("/health")
def health():
    return {
        "ok": True,
        "model_loaded": _model is not None,
        "fp16": FP16,
        "voices_dir": str(VOICES_DIR),
        "voice_count": len(manifest().get("voices", [])),
    }


@app.get("/voices")
def list_voices():
    """List available voice presets. Returns the full voices.json content so
    callers can see prompt_text, gender, tone, and license per voice."""
    return manifest()


class TtsRequest(BaseModel):
    """Request body for the convenience POST /tts endpoint.

    Mode auto-selection:
      - instruct present and non-empty → mode = 'instruct'
      - '[laughter]' in text             → mode = 'cross_lingual'
      - else                             → mode = 'zero_shot'
    Caller can force a mode via the 'mode' field.
    """
    text: str
    voice_id: str
    instruct: Optional[str] = None       # e.g. "请非常开心地说一句话。"
    prompt_text: Optional[str] = None    # override the manifest-resolved one
    mode: Optional[str] = None           # zero_shot | instruct | cross_lingual


def _pick_mode(req: TtsRequest) -> str:
    if req.mode:
        if req.mode not in ("zero_shot", "instruct", "cross_lingual"):
            raise HTTPException(400, f"invalid mode: {req.mode}")
        return req.mode
    if req.instruct:
        return "instruct"
    if "[laughter]" in req.text:
        return "cross_lingual"
    return "zero_shot"


@app.post("/tts")
async def tts(req: TtsRequest):
    """Voice-id-based convenience wrapper. Looks up wav + prompt_text from the
    on-disk voice library so callers don't have to upload a ref every call,
    nor know that prompt_text must match the wav's transcript."""
    wav_path, default_pt = lookup_voice(req.voice_id)
    prompt_text = req.prompt_text or default_pt
    mode = _pick_mode(req)

    if mode == "zero_shot":
        chunks = list(model().inference_zero_shot(
            req.text, prompt_text, wav_path, stream=False))
    elif mode == "instruct":
        wrapped = _wrap_instruct(req.instruct or "")
        chunks = list(model().inference_instruct2(
            req.text, wrapped, wav_path, stream=False))
    else:  # cross_lingual
        chunks = list(model().inference_cross_lingual(
            req.text, wav_path, stream=False))
    return Response(content=_wav_bytes(chunks), media_type="audio/wav")


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


# CosyVoice2's instruct2 is only trained on prompts wrapped in
# "You are a helpful assistant. <instr>。<|endofprompt|>" (see
# CosyVoice/cosyvoice/utils/common.py instruct_list). Calling it without the
# delimiter token makes the model treat the whole instruct+text as text-to-read
# (verified by A/B: same prompt "用开心地说" gave a robotic read-back without
# wrap, real emotion with wrap). Auto-wrap is the safety net so callers can
# just pass natural language like "请非常开心地说一句话。".
_INSTRUCT_PFX = "You are a helpful assistant. "
_INSTRUCT_SFX = "<|endofprompt|>"


def _wrap_instruct(raw: str) -> str:
    s = raw.strip()
    if _INSTRUCT_SFX in s:
        return s  # caller wrapped it already
    if not s.startswith(_INSTRUCT_PFX):
        s = _INSTRUCT_PFX + s
    if not s.endswith(_INSTRUCT_SFX):
        s = s + _INSTRUCT_SFX
    return s


@app.post("/tts/cross_lingual")
async def cross_lingual(
    tts_text: str = Form(...),
    prompt_wav: UploadFile = File(...),
):
    """Use this for tts_text containing inline event tokens like
    [laughter] / [breath] / [sigh] — they're only fully decoded via the
    cross_lingual frontend. The zero_shot path partially recognizes
    [laughter] (produces a brief "哈") but ignores the rest. No prompt_text
    needed since cross_lingual doesn't condition on it.
    """
    path = _spool(await prompt_wav.read())
    try:
        chunks = list(
            model().inference_cross_lingual(tts_text, path, stream=False)
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
        wrapped = _wrap_instruct(instruct)
        chunks = list(
            model().inference_instruct2(tts_text, wrapped, path, stream=False)
        )
        return Response(content=_wav_bytes(chunks), media_type="audio/wav")
    finally:
        os.unlink(path)


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=int(os.environ.get("TTS_PORT", "8095")))
