"""FunASR ASR service (P0 + streaming VAD).

Contract (model-agnostic): a WebSocket session receives 16 kHz mono PCM
(int16 LE) binary frames. The service runs a *streaming* FSMN-VAD over the
incoming audio and, whenever a speech segment ends (the speaker pauses),
recognizes just that segment with offline Paraformer (+punctuation) and emits
`{"type":"segment", text, t_start, t_end}` immediately — without waiting for
the client to stop. `{"type":"flush"}` finalizes any pending segment then
emits `{"type":"done"}`. `{"type":"reset"}` clears state.

Model selection is purely via environment (ASR_*_DIR) so swapping
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
VAD = os.environ["ASR_VAD_DIR"]
PUNC = os.environ.get("ASR_PUNC_DIR") or None
DEVICE = os.environ.get("ASR_DEVICE", "cuda")
PORT = int(os.environ.get("ASR_PORT", "9100"))
# Streaming VAD chunk in ms (how often we poll VAD for endpoints).
VAD_CHUNK_MS = int(os.environ.get("ASR_VAD_CHUNK_MS", "200"))
# Silence (ms) required to END a speech segment. Larger => fewer, longer
# segments (short pauses won't split a sentence). Default 800 is too eager.
VAD_MAX_END_SIL = int(os.environ.get("ASR_VAD_MAX_END_SIL", "1500"))
SR = 16000
SAMPLES_PER_MS = SR // 1000

print(f"[asr] loading vad={VAD} (streaming) asr={PARAFORMER} punc={PUNC} "
      f"device={DEVICE}", flush=True)
VAD_MODEL = AutoModel(model=VAD, device=DEVICE, disable_update=True,
                      max_end_silence_time=VAD_MAX_END_SIL)
ASR_MODEL = AutoModel(model=PARAFORMER, punc_model=PUNC, device=DEVICE,
                      disable_update=True)
print("[asr] models ready", flush=True)


class Session:
    """Per-connection streaming state."""

    def __init__(self):
        self.buf = np.zeros(0, dtype=np.float32)  # whole session audio @16k
        self.vad_cache = {}
        self.pending = bytearray()  # bytes not yet fed to VAD
        self.cur_start_ms = None    # open segment start (ms from stream start)

    def add_pcm(self, raw: bytes):
        self.pending += raw

    def _take_chunk(self):
        """Pop one VAD_CHUNK_MS chunk of float32 samples, or None."""
        need = VAD_CHUNK_MS * SAMPLES_PER_MS * 2  # int16 bytes
        if len(self.pending) < need:
            return None
        chunk = bytes(self.pending[:need])
        del self.pending[:need]
        f = np.frombuffer(chunk, dtype="<i2").astype(np.float32) / 32768.0
        self.buf = np.concatenate([self.buf, f])
        return f

    def _flush_tail(self):
        """Return remaining pending audio as float32 (for is_final)."""
        if not self.pending:
            return np.zeros(0, dtype=np.float32)
        f = np.frombuffer(bytes(self.pending), dtype="<i2").astype(np.float32) / 32768.0
        self.pending.clear()
        self.buf = np.concatenate([self.buf, f])
        return f

    def slice_ms(self, beg_ms: int, end_ms: int) -> np.ndarray:
        a = max(0, beg_ms * SAMPLES_PER_MS)
        b = min(len(self.buf), end_ms * SAMPLES_PER_MS)
        return self.buf[a:b] if b > a else np.zeros(0, dtype=np.float32)


def recognize(seg: np.ndarray) -> str:
    if seg.size == 0:
        return ""
    res = ASR_MODEL.generate(input=seg, batch_size_s=300)
    return res[0].get("text", "") if res else ""


async def emit_segment(ws, text, beg_ms, end_ms):
    text = (text or "").strip()
    dur = (end_ms - beg_ms) if (beg_ms is not None and end_ms is not None) else -1
    print(f"[asr][seg] beg={beg_ms} end={end_ms} dur={dur}ms "
          f"{'DROP(empty)' if not text else ''} text={text!r}", flush=True)
    if not text:
        return
    await ws.send(json.dumps({
        "type": "segment", "text": text,
        "t_start": beg_ms / 1000.0 if beg_ms is not None else None,
        "t_end": end_ms / 1000.0 if end_ms is not None else None,
    }, ensure_ascii=False))


async def feed_vad(ws, s: Session, chunk: np.ndarray, is_final: bool):
    """Feed one chunk to streaming VAD; emit any completed segments.

    FSMN-VAD streaming returns value = list of [beg, end] (ms from stream
    start); a lone [beg, -1] opens a segment, [-1, end] closes it, [beg, end]
    is a complete one.
    """
    if chunk.size == 0 and not is_final:
        return
    res = VAD_MODEL.generate(input=chunk, cache=s.vad_cache,
                             is_final=is_final, chunk_size=VAD_CHUNK_MS)
    if not res:
        return
    val = res[0].get("value", [])
    if val:
        print(f"[asr][vad] is_final={is_final} value={val} "
              f"open={s.cur_start_ms}", flush=True)
    for beg, end in val:
        if beg >= 0 and end == -1:
            s.cur_start_ms = beg
        elif beg == -1 and end >= 0 and s.cur_start_ms is not None:
            seg = s.slice_ms(s.cur_start_ms, end)
            await emit_segment(ws, recognize(seg), s.cur_start_ms, end)
            s.cur_start_ms = None
        elif beg >= 0 and end >= 0:
            seg = s.slice_ms(beg, end)
            await emit_segment(ws, recognize(seg), beg, end)
            s.cur_start_ms = None


async def handle(ws):
    s = Session()
    async for msg in ws:
        if isinstance(msg, (bytes, bytearray)):
            s.add_pcm(bytes(msg))
            while True:
                chunk = s._take_chunk()
                if chunk is None:
                    break
                await feed_vad(ws, s, chunk, is_final=False)
            continue
        # text control
        try:
            t = json.loads(msg).get("type")
        except Exception:
            continue
        if t == "reset":
            s = Session()
        elif t == "flush":
            tail = s._flush_tail()
            await feed_vad(ws, s, tail, is_final=True)
            # any still-open segment -> recognize up to end of buffer
            if s.cur_start_ms is not None:
                end_ms = len(s.buf) // SAMPLES_PER_MS
                seg = s.slice_ms(s.cur_start_ms, end_ms)
                await emit_segment(ws, recognize(seg), s.cur_start_ms, end_ms)
                s.cur_start_ms = None
            await ws.send(json.dumps({"type": "done"}))


async def main():
    print(f"[asr] listening on :{PORT} (vad chunk {VAD_CHUNK_MS}ms)", flush=True)
    async with websockets.serve(handle, "0.0.0.0", PORT, max_size=None):
        await asyncio.Future()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        sys.exit(0)
