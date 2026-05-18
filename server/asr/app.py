"""FunASR ASR service (P0).

Contract (model-agnostic): a WebSocket session receives 16 kHz mono PCM
(int16, little-endian) binary frames, then a text control `{"type":"flush"}`
triggers VAD + ASR + punctuation over the buffered audio and emits one
`{"type":"segment", ...}` per VAD segment, followed by `{"type":"done"}`.

Model is selected purely by environment (ASR_*_DIR) so swapping
Paraformer / SenseVoice / faster-whisper later does not touch this code's
interface, the orchestrator, the protocol, or the client.
"""
import asyncio
import json
import os
import sys

import numpy as np
import websockets
from funasr import AutoModel

PARAFORMER = os.environ["ASR_PARAFORMER_DIR"]
VAD = os.environ.get("ASR_VAD_DIR") or None
PUNC = os.environ.get("ASR_PUNC_DIR") or None
DEVICE = os.environ.get("ASR_DEVICE", "cuda")
PORT = int(os.environ.get("ASR_PORT", "9100"))

print(f"[asr] loading model={PARAFORMER} vad={VAD} punc={PUNC} device={DEVICE}",
      flush=True)
MODEL = AutoModel(model=PARAFORMER, vad_model=VAD, punc_model=PUNC,
                  device=DEVICE, disable_update=True)
print("[asr] model ready", flush=True)


def pcm16_to_f32(buf: bytes) -> np.ndarray:
    return np.frombuffer(buf, dtype="<i2").astype(np.float32) / 32768.0


async def handle(ws):
    chunks: list[bytes] = []
    async for msg in ws:
        if isinstance(msg, (bytes, bytearray)):
            chunks.append(bytes(msg))
            continue
        # text control
        try:
            ctrl = json.loads(msg)
        except Exception:
            await ws.send(json.dumps({"type": "error",
                                      "message": "bad control frame"}))
            continue
        t = ctrl.get("type")
        if t == "reset":
            chunks.clear()
        elif t == "flush":
            audio = pcm16_to_f32(b"".join(chunks))
            chunks.clear()
            if audio.size == 0:
                await ws.send(json.dumps({"type": "done"}))
                continue
            try:
                res = MODEL.generate(input=audio, batch_size_s=300,
                                     is_final=True)
            except Exception as e:  # noqa: BLE001
                await ws.send(json.dumps({"type": "error",
                                          "message": f"asr failed: {e}"}))
                continue
            for r in res:
                await ws.send(json.dumps({
                    "type": "segment",
                    "text": r.get("text", ""),
                    # FunASR may not always return timestamps; keep optional
                    "t_start": r.get("start"),
                    "t_end": r.get("end"),
                }, ensure_ascii=False))
            await ws.send(json.dumps({"type": "done"}))
        else:
            await ws.send(json.dumps({"type": "error",
                                      "message": f"unknown control {t}"}))


async def main():
    print(f"[asr] listening on :{PORT}", flush=True)
    async with websockets.serve(handle, "0.0.0.0", PORT, max_size=None):
        await asyncio.Future()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        sys.exit(0)
