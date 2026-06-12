"""FunASR ASR service (P0 + streaming VAD).

Contract (model-agnostic): a WebSocket session receives 16 kHz mono PCM
(int16 LE) binary frames. The service runs a *streaming* FSMN-VAD over the
incoming audio and, whenever a speech segment ends (the speaker pauses),
recognizes just that segment with offline Paraformer (+punctuation) and emits
`{"type":"segment", text, t_start, t_end}` immediately — without waiting for
the client to stop. `{"type":"flush"}` finalizes any pending segment then
emits `{"type":"done"}`. `{"type":"reset"}` clears state.

Model selection is purely via environment (ASR_*_DIR) so swapping
Paraformer / SenseVoice / Whisper later does not touch this code's
interface, the orchestrator, the protocol, or the client. The active
model is hot-switchable at runtime via the orchestrator's `asr.model`.
"""
import asyncio
import json
import os
import re
import subprocess
import sys
import threading
import urllib.request

import numpy as np
import websockets
from aiohttp import web
from funasr import AutoModel

# SenseVoice emits leading meta tokens like <|zh|><|NEUTRAL|><|BGM|><|withitn|>.
# We strip them to plain text. (funasr's rich_transcription_postprocess instead
# turns emotion/audio-event tokens into emoji like 😡/🎼 — unwanted noise for a
# transcription tool; the ITN punctuation we want is already in the body text.)
_SV_TAG_RE = re.compile(r"<\|[^|]*\|>")

PARAFORMER = os.environ["ASR_PARAFORMER_DIR"]
SENSEVOICE = os.environ.get("ASR_SENSEVOICE_DIR") or None
# Whisper (FunASR-packaged, loaded via the same AutoModel torch/CUDA stack).
WHISPER_TURBO = os.environ.get("ASR_WHISPER_TURBO_DIR") or None
WHISPER_LARGE = os.environ.get("ASR_WHISPER_LARGE_DIR") or None
# Whisper decode language: "" -> auto-detect (best for mixed zh/en sessions);
# set e.g. "zh" or "en" to pin it.
WHISPER_LANGUAGE = os.environ.get("ASR_WHISPER_LANGUAGE", "").strip() or None
VAD = os.environ["ASR_VAD_DIR"]
PUNC = os.environ.get("ASR_PUNC_DIR") or None
DEVICE = os.environ.get("ASR_DEVICE", "cuda")
PORT = int(os.environ.get("ASR_PORT", "9100"))
# Streaming VAD chunk in ms (how often we poll VAD for endpoints). Smaller
# => lower endpointing jitter (segment finalize waits for the next poll), at
# the cost of more frequent (tiny) VAD generate() calls. 50ms cuts up to
# ~150ms perceived latency vs the old 200ms default.
VAD_CHUNK_MS = int(os.environ.get("ASR_VAD_CHUNK_MS", "50"))
# Silence (ms) required to END a speech segment. Larger => fewer, longer
# segments (short pauses won't split a sentence). Default 800 is too eager.
VAD_MAX_END_SIL = int(os.environ.get("ASR_VAD_MAX_END_SIL", "1500"))
# Our own sentence endpointing: VAD speech regions separated by a gap
# shorter than this are merged into ONE sentence. Only a pause >= this
# finalizes/emits. This is what actually controls "how often it splits"
# (FunASR's own max_end_silence_time is not honored in streaming).
SENTENCE_GAP_MS = int(os.environ.get("ASR_SENTENCE_GAP_MS", "1500"))
SR = 16000
SAMPLES_PER_MS = SR // 1000

# Speaker (voiceprint) gating — best model picked: CAM++ zh+en.
SPK_DIR = os.environ.get("ASR_SPK_DIR") or None
SPK_THRESHOLD = float(os.environ.get("ASR_SPK_THRESHOLD", "0.35"))
# True  -> only recognize enrolled+enabled speakers (drop others)
# False -> recognize everyone (gating off, even if voiceprints exist)
GATE_TO_ENROLLED = os.environ.get("ASR_GATE_TO_ENROLLED", "1") not in ("0", "off", "false")
ORCH_BASE = os.environ.get("ORCH_BASE", "http://orchestrator:8090")
HTTP_PORT = int(os.environ.get("ASR_HTTP_PORT", "9101"))
VP_REFRESH_SEC = int(os.environ.get("ASR_VP_REFRESH_SEC", "15"))

_WHISPER_KINDS = ("whisper-turbo", "whisper-large-v3")
VALID_ASR_KINDS = ("paraformer", "sensevoice") + _WHISPER_KINDS

# 领域热词(由 orchestrator 的 asr.hotwords 配置驱动,每 15s 轮询):
#   HOTWORDS_PARAFORMER  -> 空格分隔字符串,作为 Paraformer 的 hotword=
#   HOTWORDS_WHISPER     -> 用作 Whisper DecodingOptions.prompt 的 initial_prompt
# SenseVoice 不支持热词偏置,自动跳过(仍可在 LLM 润色侧兜底,由 orchestrator 处理)。
HOTWORDS_PARAFORMER: str = ""
HOTWORDS_WHISPER: str = ""


def _parse_hotwords(raw: str):
    """从 textarea 文本解析热词,返回 (paraformer_str, whisper_str)。

    每行 "词" 或 "词 权重"。
    - Paraformer 接受 "w1 w2 ..." 形式(原生 hotword 接口)。
    - Whisper 走 initial_prompt(自然语言"上下文"),孤立词偏置很弱,所以
      包装成一句完整中文提示句,显著提升对术语的命中率。
    """
    words: list[str] = []
    for line in (raw or "").splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        w = s.split()[0]
        if w:
            words.append(w)
    if not words:
        return ("", "")
    whisper_prompt = "本段语音内容涉及以下术语,如出现同音字应优先识别为:" \
        + "、".join(words) + "。"
    return (" ".join(words), whisper_prompt)

# Secondary recognizer kind for side-by-side comparison. Picked by the
# orchestrator's `asr.secondary_model` config (hot-switchable). Per-session
# opt-in via the hello's `want_secondary` flag — keeps default sessions at
# the original VRAM footprint. Default initial kind: sensevoice if its
# weights env is set, otherwise disabled until the management console
# picks one.
_DEFAULT_SECONDARY = "sensevoice" if SENSEVOICE else ""
SECONDARY_KIND: str = os.environ.get("ASR_SECONDARY_MODEL", _DEFAULT_SECONDARY).strip()
SECONDARY_MODEL = None  # lazy: only built once the first opt-in session arrives


def _build_asr(kind: str):
    """Construct the offline recognizer for `kind`.

    kind ∈ paraformer | sensevoice | whisper-turbo | whisper-large-v3.
    Heavy (loads weights onto GPU); called at startup and on hot-switch.
    """
    if kind == "sensevoice":
        if not SENSEVOICE:
            raise RuntimeError("ASR_SENSEVOICE_DIR not set")
        return AutoModel(model=SENSEVOICE, device=DEVICE, disable_update=True)
    if kind in _WHISPER_KINDS:
        turbo = kind == "whisper-turbo"
        d = WHISPER_TURBO if turbo else WHISPER_LARGE
        if not d:
            env = "ASR_WHISPER_TURBO_DIR" if turbo else "ASR_WHISPER_LARGE_DIR"
            raise RuntimeError(f"{env} not set")
        # No vad_model: our streaming FSMN-VAD already cuts sentences, and
        # recognize() feeds Whisper one (<=30s) segment at a time.
        return AutoModel(model=d, device=DEVICE, disable_update=True)
    return AutoModel(model=PARAFORMER, punc_model=PUNC, device=DEVICE,
                     disable_update=True)


print(f"[asr] loading vad={VAD} (streaming) asr={PARAFORMER} punc={PUNC} "
      f"spk={SPK_DIR} device={DEVICE}", flush=True)
VAD_MODEL = AutoModel(model=VAD, device=DEVICE, disable_update=True,
                      max_end_silence_time=VAD_MAX_END_SIL)
# Currently loaded recognizer. ASR_KIND is reconciled at runtime against the
# orchestrator's `asr.model` config (hot-switch, see _refresh_asr_config).
ASR_KIND = "paraformer"
ASR_MODEL = _build_asr(ASR_KIND)
SPK_MODEL = (AutoModel(model=SPK_DIR, device=DEVICE, disable_update=True)
             if SPK_DIR else None)
print(f"[asr] models ready (asr={ASR_KIND} speaker="
      f"{'on' if SPK_MODEL else 'off'})", flush=True)

# Enabled voiceprints pulled from the orchestrator: list[(name, np.ndarray)].
ENABLED_VPS: list = []


def spk_embed(audio: np.ndarray):
    """Compute a speaker embedding for 16k mono f32 audio, or None."""
    if SPK_MODEL is None or audio.size < SR // 2:  # need >= ~0.5s
        return None
    try:
        res = SPK_MODEL.generate(input=audio)
    except Exception as e:  # noqa: BLE001
        print(f"[asr][spk] embed failed: {e}", flush=True)
        return None
    if not res:
        return None
    emb = res[0].get("spk_embedding")
    if emb is None:
        emb = res[0].get("embedding")
    if emb is None:
        return None
    if hasattr(emb, "detach"):  # torch tensor (CAM++ returns it on cuda)
        emb = emb.detach().cpu().numpy()
    v = np.asarray(emb, dtype=np.float32).reshape(-1)
    n = np.linalg.norm(v)
    return v / n if n > 0 else None


def cosine(a, b) -> float:
    if a is None or b is None or a.shape != b.shape:
        return -1.0
    return float(np.dot(a, b))  # both unit-normalized


def best_speaker(audio: np.ndarray):
    """(name, score) of best enabled voiceprint, or (None, -1)."""
    if not ENABLED_VPS:
        return (None, 1.0)  # gating disabled -> accept all
    emb = spk_embed(audio)
    if emb is None:
        return ("", 1.0)  # embed failed -> fail open (don't drop)
    best_n, best_s = None, -1.0
    for name, vp in ENABLED_VPS:
        s = cosine(emb, vp)
        if s > best_s:
            best_n, best_s = name, s
    return (best_n, best_s)


def _decode_audio(raw: bytes) -> np.ndarray:
    """Decode arbitrary audio bytes (webm/opus/wav/...) to 16k mono f32."""
    p = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", "pipe:0", "-ac", "1",
         "-ar", str(SR), "-f", "f32le", "pipe:1"],
        input=raw, capture_output=True,
    )
    if p.returncode != 0:
        raise RuntimeError(p.stderr.decode("utf-8", "ignore")[:200])
    return np.frombuffer(p.stdout, dtype=np.float32).copy()


def _refresh_voiceprints():
    """Pull enabled voiceprints from the orchestrator (blocking)."""
    global ENABLED_VPS
    try:
        with urllib.request.urlopen(f"{ORCH_BASE}/api/voiceprints", timeout=5) as r:
            data = json.loads(r.read().decode())
        vps = []
        for item in data:
            v = np.asarray(item.get("embedding", []), dtype=np.float32).reshape(-1)
            n = np.linalg.norm(v)
            if n > 0:
                vps.append((item.get("name", ""), v / n))
        ENABLED_VPS = vps
    except Exception:
        pass  # keep last known set on transient failure


def _refresh_asr_config():
    """Pull runtime-tunable config from the orchestrator and reconcile.

    Threshold/gap apply immediately. `model` / `secondary_model` trigger a
    hot-switch: we rebuild on a poll thread and atomically swap the globals.
    An in-progress recognize() snapshots the old model, so live sessions are
    not interrupted (the new model takes effect from the next sentence).
    Secondary model is lazy — only built once a session actually opts in.
    """
    global SPK_THRESHOLD, SENTENCE_GAP_MS, ASR_MODEL, ASR_KIND
    global GATE_TO_ENROLLED, SECONDARY_KIND, SECONDARY_MODEL
    global HOTWORDS_PARAFORMER, HOTWORDS_WHISPER
    try:
        with urllib.request.urlopen(f"{ORCH_BASE}/api/asr-config", timeout=5) as r:
            d = json.loads(r.read().decode())
    except Exception:
        return  # keep current values on transient failure
    if "spk_threshold" in d:
        SPK_THRESHOLD = float(d["spk_threshold"])
    if "sentence_gap_ms" in d:
        SENTENCE_GAP_MS = int(d["sentence_gap_ms"])
    if "gate_to_enrolled" in d:
        GATE_TO_ENROLLED = str(d["gate_to_enrolled"]).strip().lower() \
            not in ("0", "off", "false", "no")
    if "hotwords" in d:
        HOTWORDS_PARAFORMER, HOTWORDS_WHISPER = _parse_hotwords(
            str(d.get("hotwords") or "")
        )
    want = str(d.get("model", "")).strip().lower()
    if want and want in VALID_ASR_KINDS and want != ASR_KIND:
        print(f"[asr][cfg] switching ASR model {ASR_KIND} -> {want} ...",
              flush=True)
        try:
            m = _build_asr(want)
        except Exception as e:  # noqa: BLE001
            print(f"[asr][cfg] switch to {want} FAILED: {e} "
                  f"(keeping {ASR_KIND})", flush=True)
            return  # retried on next poll
        ASR_MODEL, ASR_KIND = m, want
        print(f"[asr][cfg] ASR model now: {ASR_KIND}", flush=True)
    # Secondary recognizer: kind change either swaps the loaded model (if
    # already built) or just updates the desired kind (built on first opt-in).
    sec = str(d.get("secondary_model", "")).strip().lower()
    if sec != SECONDARY_KIND and (not sec or sec in VALID_ASR_KINDS):
        if sec and sec == ASR_KIND:
            print(f"[asr][cfg] secondary={sec} matches primary; skipping "
                  f"(comparison would be redundant)", flush=True)
        elif not sec:
            SECONDARY_KIND = ""
            SECONDARY_MODEL = None
            print("[asr][cfg] secondary model disabled", flush=True)
        elif SECONDARY_MODEL is None:
            SECONDARY_KIND = sec
            print(f"[asr][cfg] secondary model set to {sec} (lazy-load)",
                  flush=True)
        else:
            print(f"[asr][cfg] switching secondary {SECONDARY_KIND} -> {sec} ...",
                  flush=True)
            try:
                m = _build_asr(sec)
            except Exception as e:  # noqa: BLE001
                print(f"[asr][cfg] secondary switch to {sec} FAILED: {e} "
                      f"(keeping {SECONDARY_KIND})", flush=True)
                return
            SECONDARY_MODEL, SECONDARY_KIND = m, sec
            print(f"[asr][cfg] secondary model now: {SECONDARY_KIND}", flush=True)


# Serializes secondary weight loading: the session-config preheat and the
# first finalized segment may both call _ensure_secondary_loaded from
# executor threads concurrently — without the lock both would build a copy.
_SECONDARY_BUILD_LOCK = threading.Lock()


def _ensure_secondary_loaded():
    """Build the secondary recognizer on first opt-in. Returns (model, kind)
    or (None, "") if no secondary is configured / build failed."""
    global SECONDARY_MODEL, SECONDARY_KIND
    with _SECONDARY_BUILD_LOCK:
        if not SECONDARY_KIND:
            return (None, "")
        if SECONDARY_MODEL is not None:
            return (SECONDARY_MODEL, SECONDARY_KIND)
        if SECONDARY_KIND == ASR_KIND:
            return (None, "")  # avoid loading a duplicate of the primary
        print(f"[asr][sec] lazy-loading secondary model {SECONDARY_KIND} ...",
              flush=True)
        try:
            SECONDARY_MODEL = _build_asr(SECONDARY_KIND)
        except Exception as e:  # noqa: BLE001
            print(f"[asr][sec] load failed: {e} — disabling secondary",
                  flush=True)
            SECONDARY_KIND = ""
            SECONDARY_MODEL = None
            return (None, "")
        print(f"[asr][sec] secondary ready: {SECONDARY_KIND}", flush=True)
        return (SECONDARY_MODEL, SECONDARY_KIND)


def recognize_with(seg: np.ndarray, model, kind: str) -> str:
    """Same as `recognize` but on an explicit (model, kind) pair — used to
    drive the secondary recognizer without disturbing the primary globals."""
    if seg.size == 0 or model is None or not kind:
        return ""
    if kind == "sensevoice":
        # SenseVoice 走 CTC 解码,FunASR 未暴露热词偏置接口 — 跳过(LLM 兜底)。
        res = model.generate(input=seg, cache={}, language="auto",
                             use_itn=True, batch_size_s=300)
        if not res:
            return ""
        return _SV_TAG_RE.sub("", res[0].get("text", "")).strip()
    if kind in _WHISPER_KINDS:
        opts = {
            "task": "transcribe",
            "language": WHISPER_LANGUAGE,
            "beam_size": None,
            "fp16": DEVICE.startswith("cuda"),
            "without_timestamps": True,
            "prompt": HOTWORDS_WHISPER or None,
        }
        res = model.generate(input=seg, DecodingOptions=opts, batch_size_s=0)
        if not res:
            return ""
        return _SV_TAG_RE.sub("", res[0].get("text", "")).strip()
    # Paraformer:原生 hotword 接口,空串时 FunASR 不做偏置。
    kwargs = {"batch_size_s": 300}
    if HOTWORDS_PARAFORMER:
        kwargs["hotword"] = HOTWORDS_PARAFORMER
    res = model.generate(input=seg, **kwargs)
    return res[0].get("text", "") if res else ""


async def voiceprint_loop():
    while True:
        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, _refresh_voiceprints)
        await loop.run_in_executor(None, _refresh_asr_config)
        sec_state = (
            f"loaded={SECONDARY_KIND}"
            if SECONDARY_MODEL is not None
            else (f"pending={SECONDARY_KIND}" if SECONDARY_KIND else "off")
        )
        hw_n = len(HOTWORDS_PARAFORMER.split()) if HOTWORDS_PARAFORMER else 0
        print(f"[asr][cfg] model={ASR_KIND} secondary={sec_state} "
              f"voiceprints={len(ENABLED_VPS)} "
              f"gate={'on' if GATE_TO_ENROLLED else 'off'} "
              f"spk_thr={SPK_THRESHOLD} gap={SENTENCE_GAP_MS}ms "
              f"hotwords={hw_n}", flush=True)
        await asyncio.sleep(VP_REFRESH_SEC)


async def http_embed(request: web.Request) -> web.Response:
    """POST audio bytes -> {'embedding':[...]} (for enrollment)."""
    raw = await request.read()
    try:
        audio = _decode_audio(raw)
    except Exception as e:  # noqa: BLE001
        return web.json_response({"error": f"decode failed: {e}"}, status=400)
    emb = spk_embed(audio)
    if emb is None:
        return web.json_response({"error": "embedding failed"}, status=400)
    return web.json_response({"embedding": emb.tolist()})


def _vad_regions_offline(audio: np.ndarray) -> list[tuple[int, int]]:
    """One-shot offline FSMN-VAD over a whole clip → flat [(beg_ms,end_ms), ...].

    Same model as the streaming path; cache is fresh so it doesn't interfere
    with live sessions. Streaming VAD emits a mix of [beg,-1] / [-1,end] /
    [beg,end] events — here we fold them into closed regions and then merge
    neighbours separated by < SENTENCE_GAP_MS so the segment partitioning
    mirrors what the realtime pipeline would produce on the same audio.
    """
    if audio.size == 0:
        return []
    res = VAD_MODEL.generate(input=audio, cache={}, is_final=True,
                             chunk_size=VAD_CHUNK_MS, disable_pbar=True)
    val = res[0].get("value", []) if res else []
    regions: list[tuple[int, int]] = []
    cur_beg: int | None = None
    for beg, end in val:
        if beg >= 0 and end == -1:
            if cur_beg is None:
                cur_beg = beg
        elif beg == -1 and end >= 0:
            if cur_beg is not None:
                regions.append((cur_beg, end))
                cur_beg = None
        elif beg >= 0 and end >= 0:
            regions.append((beg, end))
    merged: list[tuple[int, int]] = []
    for beg, end in regions:
        if merged and beg - merged[-1][1] < SENTENCE_GAP_MS:
            merged[-1] = (merged[-1][0], end)
        else:
            merged.append((beg, end))
    return merged


def _transcribe_blocking(audio: np.ndarray, want_vad: bool) -> dict:
    """Heavy path: VAD + recognize (or whole-clip recognize). Returns the
    JSON body shape directly. Runs on the executor so the event loop stays
    free for live ws sessions on :9100."""
    model, kind = ASR_MODEL, ASR_KIND
    if not want_vad:
        text = recognize_with(audio, model, kind).strip()
        return {"text": text, "segments": [], "model": kind}
    segments: list[dict] = []
    pieces: list[str] = []
    for beg_ms, end_ms in _vad_regions_offline(audio):
        a = max(0, beg_ms * SAMPLES_PER_MS)
        b = min(len(audio), end_ms * SAMPLES_PER_MS)
        if b <= a:
            continue
        text = recognize_with(audio[a:b], model, kind).strip()
        if not text:
            continue
        segments.append({
            "t_start": beg_ms / 1000.0,
            "t_end": end_ms / 1000.0,
            "text": text,
        })
        pieces.append(text)
    return {"text": "\n".join(pieces), "segments": segments, "model": kind}


async def http_transcribe(request: web.Request) -> web.Response:
    """POST multipart audio -> {text, segments, model}.

    Form fields:
      audio (file, required): wav / mp3 / mp4 / webm / … — anything ffmpeg
                              can decode. Re-encoded to 16k mono f32 in-process.
      vad   (str,  optional): "1"/"true" (default) → VAD-segmented output with
                              per-region timestamps; "0"/"false" → single
                              whole-clip recognize, segments=[].

    Model selection is **not** per-request: the active recognizer is
    governed by the orchestrator's `asr.model` config (hot-switchable),
    the same as the streaming path. Returning `model` in the body lets the
    caller stamp the transcript with what actually produced it.
    """
    if not request.content_type.startswith("multipart/"):
        return web.json_response(
            {"error": "expected multipart/form-data with an 'audio' part"},
            status=400,
        )
    audio_bytes: bytes | None = None
    want_vad = True
    try:
        reader = await request.multipart()
        async for part in reader:
            name = part.name
            if name == "audio":
                audio_bytes = await part.read(decode=False)
            elif name == "vad":
                v = (await part.text()).strip().lower()
                want_vad = v not in ("0", "false", "off", "no", "")
    except Exception as e:  # noqa: BLE001
        return web.json_response({"error": f"bad multipart: {e}"}, status=400)
    if not audio_bytes:
        return web.json_response({"error": "missing 'audio' part"}, status=400)
    try:
        audio = _decode_audio(audio_bytes)
    except Exception as e:  # noqa: BLE001
        return web.json_response({"error": f"decode failed: {e}"}, status=400)
    print(f"[asr][http] /transcribe bytes={len(audio_bytes)} "
          f"dur={len(audio)/SR:.2f}s vad={want_vad} model={ASR_KIND}",
          flush=True)
    loop = asyncio.get_event_loop()
    try:
        body = await loop.run_in_executor(
            None, _transcribe_blocking, audio, want_vad
        )
    except Exception as e:  # noqa: BLE001
        print(f"[asr][http] /transcribe failed: {e}", flush=True)
        return web.json_response({"error": f"recognize failed: {e}"}, status=500)
    return web.json_response(body)


class Session:
    """Per-connection streaming state with sentence-level endpointing."""

    def __init__(self):
        self.buf = np.zeros(0, dtype=np.float32)  # recent session audio @16k
        # Absolute ms position of buf[0] in the stream. VAD timestamps are
        # absolute from stream start; finalized audio is trimmed from the
        # head of buf (see trim_to) so long sessions don't grow unbounded —
        # this offset keeps slice_ms/now_ms in absolute terms.
        self.buf_beg_ms = 0
        self.vad_cache = {}
        self.pending = bytearray()  # bytes not yet fed to VAD
        self.speech_open = False    # VAD currently inside speech
        self.sent_beg = None        # accumulating sentence start (ms)
        self.last_end = None        # end (ms) of last closed speech region
        # Per-session opt-in for the secondary recognizer (set by the
        # orchestrator's {type:"config", want_secondary:bool} handshake).
        # Off by default so existing clients see no behaviour change.
        self.want_secondary = False
        # Background secondary-recognition tasks; awaited before `done` so the
        # client doesn't miss a trailing comparison result.
        self.sec_tasks: list = []

    def now_ms(self) -> int:
        return self.buf_beg_ms + len(self.buf) // SAMPLES_PER_MS

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
        if beg_ms < self.buf_beg_ms:
            # Shouldn't happen (trim keeps a margin past the last finalized
            # end), but clamp + log rather than return garbage.
            print(f"[asr][buf] slice beg={beg_ms} < buf_beg={self.buf_beg_ms}"
                  f" — clamped", flush=True)
        a = max(0, (beg_ms - self.buf_beg_ms) * SAMPLES_PER_MS)
        b = min(len(self.buf), (end_ms - self.buf_beg_ms) * SAMPLES_PER_MS)
        return self.buf[a:b] if b > a else np.zeros(0, dtype=np.float32)

    def trim_to(self, keep_from_ms: int):
        """Drop buffered audio older than keep_from_ms (absolute ms).

        Called after a sentence is finalized: everything up to its end has
        been recognized and will never be sliced again (the next sentence
        starts at least SENTENCE_GAP_MS later). .copy() releases the old
        backing array instead of keeping a view alive.
        """
        cut = (keep_from_ms - self.buf_beg_ms) * SAMPLES_PER_MS
        if cut <= 0:
            return
        cut = min(cut, len(self.buf))
        self.buf = self.buf[cut:].copy()
        self.buf_beg_ms += cut // SAMPLES_PER_MS


def recognize(seg: np.ndarray) -> str:
    if seg.size == 0:
        return ""
    # Snapshot the globals once: a hot-switch may swap them concurrently.
    model, kind = ASR_MODEL, ASR_KIND
    if kind == "sensevoice":
        res = model.generate(input=seg, cache={}, language="auto",
                             use_itn=True, batch_size_s=300)
        if not res:
            return ""
        return _SV_TAG_RE.sub("", res[0].get("text", "")).strip()
    if kind in _WHISPER_KINDS:
        opts = {
            "task": "transcribe",
            "language": WHISPER_LANGUAGE,  # None -> auto-detect
            "beam_size": None,
            "fp16": DEVICE.startswith("cuda"),
            "without_timestamps": True,
            "prompt": HOTWORDS_WHISPER or None,
        }
        res = model.generate(input=seg, DecodingOptions=opts, batch_size_s=0)
        if not res:
            return ""
        return _SV_TAG_RE.sub("", res[0].get("text", "")).strip()
    kwargs = {"batch_size_s": 300}
    if HOTWORDS_PARAFORMER:
        kwargs["hotword"] = HOTWORDS_PARAFORMER
    res = model.generate(input=seg, **kwargs)
    return res[0].get("text", "") if res else ""


async def emit_segment(ws, text, beg_ms, end_ms, speaker=None):
    text = (text or "").strip()
    dur = (end_ms - beg_ms) if (beg_ms is not None and end_ms is not None) else -1
    print(f"[asr][seg] beg={beg_ms} end={end_ms} dur={dur}ms spk={speaker} "
          f"{'DROP(empty)' if not text else ''} text={text!r}", flush=True)
    if not text:
        return
    msg = {
        "type": "segment", "text": text,
        "t_start": beg_ms / 1000.0 if beg_ms is not None else None,
        "t_end": end_ms / 1000.0 if end_ms is not None else None,
    }
    if speaker:
        msg["speaker"] = speaker
    await ws.send(json.dumps(msg, ensure_ascii=False))


async def _run_secondary(ws, seg: np.ndarray, beg: int, end: int):
    """Run the secondary recognizer off the event loop and emit a paired
    `secondary` event. Pairing key on the client side is (t_start,t_end) —
    same values the primary segment carries, since both share VAD output.
    Both lazy-load and inference go through `run_in_executor` — the first
    opt-in segment otherwise blocks the event loop for ~10-30s on weights
    load, which stalls all in-flight sessions on this asr instance."""
    loop = asyncio.get_event_loop()
    model, kind = await loop.run_in_executor(None, _ensure_secondary_loaded)
    if model is None:
        return
    try:
        text = await loop.run_in_executor(None, recognize_with, seg, model, kind)
    except Exception as e:  # noqa: BLE001
        print(f"[asr][sec] recognize failed ({kind}): {e}", flush=True)
        return
    text = (text or "").strip()
    print(f"[asr][sec] beg={beg} end={end} kind={kind} text={text!r}",
          flush=True)
    if not text:
        return
    msg = {
        "type": "secondary",
        "kind": kind,
        "text": text,
        "t_start": beg / 1000.0 if beg is not None else None,
        "t_end": end / 1000.0 if end is not None else None,
    }
    try:
        await ws.send(json.dumps(msg, ensure_ascii=False))
    except Exception:
        pass  # client gone; nothing to do


async def finalize(ws, s: Session):
    """Recognize and emit the accumulated sentence, then reset it."""
    if s.sent_beg is None or s.last_end is None or s.last_end <= s.sent_beg:
        s.sent_beg = None
        s.last_end = None
        return
    beg, end = s.sent_beg, s.last_end
    # .copy() decouples seg from the session buffer so trim_to below can
    # actually release the old backing array.
    seg = s.slice_ms(beg, end).copy()
    s.sent_beg = None
    s.last_end = None
    # This sentence is done; drop its audio (keep 1s margin in case VAD
    # backdates the next speech onset slightly).
    s.trim_to(end - 1000)
    text = recognize(seg)
    spk, score = best_speaker(seg)
    gated = GATE_TO_ENROLLED and bool(ENABLED_VPS)
    if gated and spk and score < SPK_THRESHOLD:
        print(f"[asr][spk] DROP non-target best={spk} score={score:.3f} "
              f"thr={SPK_THRESHOLD} text={text!r}", flush=True)
        return  # gated out: not an enrolled/enabled speaker
    # Label the speaker when a known voiceprint matches confidently —
    # informational even when gating is off.
    speaker = spk if (ENABLED_VPS and spk and score >= SPK_THRESHOLD) else None
    await emit_segment(ws, text, beg, end, speaker)
    # Fan out the same PCM slice to the secondary recognizer (if opted-in
    # for this session). Detached so primary path latency is unaffected.
    if s.want_secondary and SECONDARY_KIND:
        # Use a copy so subsequent buf growth / reset can't race the worker.
        task = asyncio.create_task(_run_secondary(ws, seg.copy(), beg, end))
        s.sec_tasks.append(task)
        # Lightly prune finished tasks so the list doesn't grow unbounded
        # in long sessions.
        s.sec_tasks = [t for t in s.sec_tasks if not t.done()]


async def feed_vad(ws, s: Session, chunk: np.ndarray, is_final: bool):
    """Streaming VAD + our own sentence endpointing.

    FSMN-VAD streaming yields value entries: [beg,-1] opens speech,
    [-1,end] closes it, [beg,end] is a complete region (ms from start).
    We merge regions separated by < SENTENCE_GAP_MS into one sentence and
    only finalize when silence since last speech reaches SENTENCE_GAP_MS
    (or on flush).
    """
    if chunk.size == 0 and not is_final:
        return
    # disable_pbar: 50ms 轮询下 tqdm 进度条会每秒刷 20 行日志。
    res = VAD_MODEL.generate(input=chunk, cache=s.vad_cache,
                             is_final=is_final, chunk_size=VAD_CHUNK_MS,
                             disable_pbar=True)
    val = res[0].get("value", []) if res else []
    if val:
        print(f"[asr][vad] is_final={is_final} value={val} "
              f"sent_beg={s.sent_beg} last_end={s.last_end}", flush=True)

    for beg, end in val:
        if beg >= 0 and end == -1:  # speech opens
            if (s.sent_beg is not None and s.last_end is not None
                    and beg - s.last_end >= SENTENCE_GAP_MS):
                await finalize(ws, s)
            if s.sent_beg is None:
                s.sent_beg = beg
            s.speech_open = True
        elif beg == -1 and end >= 0:  # speech closes
            if s.sent_beg is None:
                s.sent_beg = max(0, (s.last_end if s.last_end else end))
            s.last_end = end
            s.speech_open = False
        elif beg >= 0 and end >= 0:  # complete region
            if (s.sent_beg is not None and s.last_end is not None
                    and beg - s.last_end >= SENTENCE_GAP_MS):
                await finalize(ws, s)
            if s.sent_beg is None:
                s.sent_beg = beg
            s.last_end = end
            s.speech_open = False

    if is_final:
        if s.sent_beg is not None and s.last_end is None:
            s.last_end = s.now_ms()
        await finalize(ws, s)
        return

    # Long-enough pause since last speech -> finalize the sentence.
    if (not s.speech_open and s.sent_beg is not None
            and s.last_end is not None
            and s.now_ms() - s.last_end >= SENTENCE_GAP_MS):
        await finalize(ws, s)


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
            ctrl = json.loads(msg)
            t = ctrl.get("type")
        except Exception:
            continue
        if t == "reset":
            # Drop any in-flight secondary tasks for the previous segments
            # before throwing away the session — they belong to a different
            # logical recording from the client's perspective.
            for task in s.sec_tasks:
                task.cancel()
            s = Session()
        elif t == "config":
            # Per-session knobs negotiated with the orchestrator (currently
            # just `want_secondary`). Treated as a sticky preference; the
            # orchestrator sends it once right after connect.
            if "want_secondary" in ctrl:
                s.want_secondary = bool(ctrl.get("want_secondary"))
                print(f"[asr][cfg] session want_secondary={s.want_secondary} "
                      f"(secondary kind={SECONDARY_KIND or 'off'})", flush=True)
                # Preheat: start loading the secondary weights now instead of
                # on the first finalized segment — otherwise the first
                # comparison result lags 10-30s behind on a cold model.
                if s.want_secondary and SECONDARY_KIND and SECONDARY_MODEL is None:
                    asyncio.get_event_loop().run_in_executor(
                        None, _ensure_secondary_loaded)
        elif t == "flush":
            tail = s._flush_tail()
            await feed_vad(ws, s, tail, is_final=True)
            # Wait for any secondary recognitions queued by the just-finalised
            # segments so the client sees their comparison text before `done`.
            if s.sec_tasks:
                try:
                    await asyncio.wait_for(
                        asyncio.gather(*s.sec_tasks, return_exceptions=True),
                        timeout=15,
                    )
                except asyncio.TimeoutError:
                    print("[asr][sec] drain on flush timed out", flush=True)
                s.sec_tasks.clear()
            await ws.send(json.dumps({"type": "done"}))


async def main():
    # /embed HTTP (enrollment) + /transcribe (offline transcription) on HTTP_PORT.
    # Default 1 MiB body limit blows up on multipart mp4 (抖音单条 5-50 MiB)
    # — bump to 256 MiB which covers any realistic short-video upload.
    httpd = web.Application(client_max_size=256 * 1024 * 1024)
    httpd.router.add_post("/embed", http_embed)
    httpd.router.add_post("/transcribe", http_transcribe)
    runner = web.AppRunner(httpd)
    await runner.setup()
    await web.TCPSite(runner, "0.0.0.0", HTTP_PORT).start()
    print(f"[asr] ws :{PORT} (vad {VAD_CHUNK_MS}ms) | "
          f"http :{HTTP_PORT} /embed /transcribe", flush=True)
    async with websockets.serve(handle, "0.0.0.0", PORT, max_size=None):
        await asyncio.gather(voiceprint_loop(), asyncio.Future())


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        sys.exit(0)
