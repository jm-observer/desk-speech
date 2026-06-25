"""诊断单个 dump:用其 json 里的 ref_text 重跑 word 粒度,打印逐音素。
用法:python diag_one.py /debug/assess-XXXX.wav"""
import glob
import json
import sys

import gop

wav = sys.argv[1] if len(sys.argv) > 1 else sorted(glob.glob("/debug/*.wav"))[-1]
js = wav[:-4] + ".json"
ref = json.load(open(js))["ref_text"]
print("file:", wav.split("/")[-1], " ref:", ref)
r = gop.assess(wav, ref, {"granularity": "word"})
print("句分=%.3f bad=%s" % (r["sentence_score"], r.get("bad_phone_count")))
for wd in r["words"]:
    phs = " ".join("%s=%.2f%s" % (p["ph"], p["score"], p["pron_status"][0]) for p in wd.get("phones", []))
    print("  %-10s %.2f %-5s | %s" % (wd["ref"], wd.get("score", 0), wd["pron_status"], phs))
