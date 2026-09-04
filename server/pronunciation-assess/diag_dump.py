"""临时诊断:读 /debug/*.wav,打印音频统计 + word/phone 级评测,定位"分数过不去"。"""
import glob
import wave

import numpy as np

import gop

REF = "I need a break"

for f in sorted(glob.glob("/debug/*.wav")):
    with wave.open(f, "rb") as w:
        n = w.getnframes()
        sr = w.getframerate()
        pcm = np.frombuffer(w.readframes(n), dtype=np.int16).astype(np.float32) / 32768.0
    dur = n / sr
    rms = float(np.sqrt(np.mean(pcm ** 2)))
    peak = float(np.max(np.abs(pcm)))
    name = f.split("/")[-1]
    print("\n=== %s  %.2fs sr=%d peak=%.2f rms=%.3f ===" % (name, dur, sr, peak, rms))
    r = gop.assess(f, REF, {"granularity": "word"})
    print("  sentence=%.3f bad=%s" % (r["sentence_score"], r.get("bad_phone_count")))
    for wd in r["words"]:
        phs = " ".join(
            "%s=%.2f%s" % (p["ph"], p["score"], p["pron_status"][0])
            for p in wd.get("phones", [])
        )
        print("  %-7s %.2f %-5s | %s" % (wd["ref"], wd.get("score", 0), wd["pron_status"], phs))
