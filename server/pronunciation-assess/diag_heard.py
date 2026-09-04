"""诊断:模型在这段录音里**逐帧听到了什么**(CTC greedy),与强制对齐给出的音素位置并排看。
回答「分数低是用户读错了,还是对齐排错了」——这是所有判分争议的第一现场。

用法(容器内):python3 /app/diag_heard.py /debug/assess-xxxx.json
"""
import json
import sys
import wave

import numpy as np
import torch
import torchaudio
from g2p_en import G2p

import gop

gop._load_model()
_g2p = G2p()

jf = sys.argv[1]
ref = json.load(open(jf))["ref_text"]
wavf = jf[:-5] + ".wav"
with wave.open(wavf, "rb") as w:
    pcm = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0
    dur = w.getnframes() / w.getframerate()
wav = torch.from_numpy(pcm)
with torch.no_grad():
    iv = gop._processor(wav.numpy(), sampling_rate=16000, return_tensors="pt").input_values.to(gop.DEVICE)
    em_t = torch.log_softmax(gop._model(iv).logits[0], dim=-1).cpu()
em = em_t.numpy()
T = em.shape[0]
fs = (wav.shape[0] / T) / 16000
print("%s  ref=%s  时长=%.2fs  帧=%d(%.0fms/帧)" % (wavf.split("/")[-1], ref, dur, T, fs * 1000))

# ---- CTC greedy:模型自己认为听到的音素串(合并重复 + 去 blank),带时间 ----
argmax = em.argmax(axis=1)
heard = []
prev = -1
for t, a in enumerate(argmax):
    if a != prev and a != gop._blank_id:
        heard.append((round(t * fs, 2), gop._vocab_inv.get(int(a), "?"), float(em[t, a])))
    prev = a
print("\n模型听到(greedy):")
print("  " + "  ".join("%s@%.2f" % (tok, t) for t, tok, _ in heard))

# ---- 期望音素:每个在全局的最佳峰 + 峰值 ----
words = gop.tokenize_words(ref)
print("\n期望音素的全局最佳峰(模型在整段里最像这个音的时刻):")
for w in words:
    row = []
    for ph in gop._g2p_word(w, _g2p):
        ids = gop._resolve_token_ids(ph)
        if not ids:
            row.append("%s:未登录" % ph); continue
        best = max(ids, key=lambda c: float(em[:, c].max()))
        row.append("%s@%.2f(%.1f)" % (gop.strip_stress(ph), int(em[:, best].argmax()) * fs,
                                      float(em[:, best].max())))
    print("  %-12s %s" % (w, "  ".join(row)))

# ---- 每帧 top1(只打非 blank 帧),看音素在时间轴上的真实排布 ----
print("\n非 blank 帧 top1(时间 → 音素 / 后验):")
line = []
for t in range(T):
    a = int(argmax[t])
    if a == gop._blank_id:
        continue
    line.append("%.2f:%s(%.1f)" % (t * fs, gop._vocab_inv.get(a, "?"), float(em[t, a])))
for i in range(0, len(line), 6):
    print("  " + "  ".join(line[i:i + 6]))
