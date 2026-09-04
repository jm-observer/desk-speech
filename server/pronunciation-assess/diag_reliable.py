"""打印每音素:对齐 span / canonical 段内峰(gop_raw) / 段内最强竞争音 + 其峰 / canonical 全局峰@帧 + 段内分数,
用真实数字调 uncertain 触发规则。用法:python diag_reliable.py /debug/xxx.wav"""
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
            t = gop._resolve_token_id(ph)
            if t is not None:
                fp.append(gop.strip_stress(ph)); fid.append(t)
    return fp, fid


with wave.open(wavf, "rb") as w:
    pcm = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0
with torch.no_grad():
    iv = gop._processor(pcm, sampling_rate=16000, return_tensors="pt").input_values.to(gop.DEVICE)
    em = torch.log_softmax(gop._model(iv).logits[0], dim=-1).cpu()
fp, fid = targets(ref)
al, sc = torchaudio.functional.forced_align(em.unsqueeze(0), torch.tensor([fid]), blank=gop._blank_id)
spans = [s for s in torchaudio.functional.merge_tokens(al[0], sc[0]) if s.token != gop._blank_id]
print("%s  ref=%s" % (wavf.split("/")[-1], ref))
print("  ph   span      gop_raw  alignScore  竞争音=peak     canon全局峰@帧")
for i, ph in enumerate(fp):
    if i >= len(spans):
        break
    sp = spans[i]; cid = fid[i]
    seg = em[sp.start:sp.end + 1]
    graw = float(seg[:, cid].max())
    pk = seg.max(dim=0).values.clone(); pk[cid] = -1e30
    if 0 <= gop._blank_id < pk.shape[0]:
        pk[gop._blank_id] = -1e30
    oid = int(pk.argmax()); oraw = float(pk[oid])
    otok = gop.model_token_to_arpabet(gop._vocab_inv.get(oid, "?"))
    gpos = int(em[:, cid].argmax()); gmax = float(em[:, cid].max())
    print("  %-4s [%3d,%3d] %7.2f  %9.2f   %-4s=%6.2f   %6.2f@%d" %
          (ph, sp.start, sp.end, graw, float(sp.score), otok, oraw, gmax, gpos))
