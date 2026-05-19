"""A/B: Paraformer vs SenseVoice on a fixed Chinese-leaning clip set.

Raw ASR only (no VAD / no orchestrator / no LLM) so we compare recognition
quality in isolation. Run inside the asr container (funasr + GPU + /models).
"""
import glob
import subprocess
import sys
import time

import numpy as np
from funasr import AutoModel
from funasr.utils.postprocess_utils import rich_transcription_postprocess

SR = 16000
PARAFORMER = "/models/speech_paraformer-large_asr_nat-zh-cn-16k-common-vocab8404-pytorch"
PUNC = "/models/punc_ct-transformer_zh-cn-common-vocab272727-pytorch"
SENSEVOICE = "/models/SenseVoiceSmall"

CLIPS = [
    PARAFORMER + "/example/asr_example.wav",
    "/models/speech_fsmn_vad_zh-cn-16k-common-pytorch/example/vad_example.wav",
    SENSEVOICE + "/example/zh.mp3",
    SENSEVOICE + "/example/yue.mp3",
    SENSEVOICE + "/example/en.mp3",
] + sorted(glob.glob(
    "/models/speech_campplus_sv_zh_en_16k-common_advanced/examples/*_cn_16k.wav"))


def load(path: str) -> np.ndarray:
    p = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-ac", "1",
         "-ar", str(SR), "-f", "f32le", "pipe:1"],
        capture_output=True,
    )
    if p.returncode != 0:
        raise RuntimeError(p.stderr.decode("utf-8", "ignore")[:200])
    return np.frombuffer(p.stdout, dtype=np.float32).copy()


print("loading models...", flush=True)
pf = AutoModel(model=PARAFORMER, punc_model=PUNC, device="cuda",
               disable_update=True)
sv = AutoModel(model=SENSEVOICE, device="cuda", disable_update=True)
print("ready\n", flush=True)


def run_pf(a):
    t = time.time()
    r = pf.generate(input=a, batch_size_s=300)
    return (r[0].get("text", "") if r else ""), time.time() - t


def run_sv(a):
    t = time.time()
    r = sv.generate(input=a, cache={}, language="auto", use_itn=True,
                    batch_size_s=300)
    txt = rich_transcription_postprocess(r[0].get("text", "")) if r else ""
    return txt, time.time() - t


for path in CLIPS:
    name = path.split("/models/")[-1]
    try:
        a = load(path)
    except Exception as e:  # noqa: BLE001
        print(f"### {name}\n  (load failed: {e})\n", flush=True)
        continue
    dur = len(a) / SR
    pf_t, pf_s = run_pf(a)
    sv_t, sv_s = run_sv(a)
    print(f"### {name}  ({dur:.1f}s audio)", flush=True)
    print(f"  PF  [{pf_s:4.1f}s] {pf_t!r}", flush=True)
    print(f"  SV  [{sv_s:4.1f}s] {sv_t!r}", flush=True)
    print(flush=True)

print("done", flush=True)
