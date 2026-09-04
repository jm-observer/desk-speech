"""实验:某些音素峰值 GOP≈0,是对齐错位(后验尖峰落在 span 之外)还是模型真低?
对每音素比较「span 内峰值」vs 全局峰值位置。用法:python diag_window.py /debug/xxx.wav"""
import glob
import json
import sys
import wave

import numpy as np
import torch
import torchaudio
from g2p_en import G2p

import gop

gop._load_model()
g2p = G2p()
wavf = sys.argv[1] if len(sys.argv) > 1 else sorted(glob.glob("/debug/*.wav"))[-1]
ref = json.load(open(wavf[:-4] + ".json"))["ref_text"]


def targets(r):
    fp, fid = [], []
    for w in gop.tokenize_words(r):
        for ph in gop._g2p_word(w, g2p):
            tid = gop._resolve_token_id(ph)
            if tid is not None:
                fp.append(gop.strip_stress(ph)); fid.append(tid)
    return fp, fid


with wave.open(wavf, "rb") as w:
    pcm = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0
wav = torch.from_numpy(pcm)
with torch.no_grad():
    iv = gop._processor(wav.numpy(), sampling_rate=16000, return_tensors="pt").input_values.to(gop.DEVICE)
    em = torch.log_softmax(gop._model(iv).logits[0], dim=-1).cpu().numpy()
fp, fid = targets(ref)
al, sc = torchaudio.functional.forced_align(
    torch.from_numpy(em).unsqueeze(0), torch.tensor([fid]), blank=gop._blank_id)
spans = [s for s in torchaudio.functional.merge_tokens(al[0], sc[0]) if s.token != gop._blank_id]
print("%s  ref=%s  T=%d" % (wavf.split("/")[-1], ref, em.shape[0]))
print("  ph    span        span峰  全局峰@帧  峰在span内?")
for i, ph in enumerate(fp):
    if i >= len(spans):
        break
    sp = spans[i]; cid = fid[i]
    inspan = float(em[sp.start:sp.end + 1, cid].max())
    gpos = int(em[:, cid].argmax()); gmax = float(em[:, cid].max())
    print("  %-4s [%3d,%3d]  %6.2f   %6.2f@%-3d  %s" %
          (ph, sp.start, sp.end, inspan, gmax, gpos, "Y" if sp.start <= gpos <= sp.end else "N(错位)"))
