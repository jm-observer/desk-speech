"""用**当前** gop.py 对历史 dump 重跑评测,与 dump 里记录的旧结果逐音素对比。
改判分规则后的第一道验证:看清"哪些音素改判了、方向对不对",而不是只看总分动了几分。

用法(容器内):python3 /app/diag_rescore.py /debug/assess-2026*.json
"""
import glob
import json
import sys

import gop

files = sorted({f for p in (sys.argv[1:] or ["/debug/assess-*.json"]) for f in glob.glob(p)})
changed = 0
for jf in files:
    try:
        old = json.load(open(jf))
    except Exception:  # noqa: BLE001
        continue
    if "result" not in old:
        continue
    ref = old["ref_text"]
    new = gop.assess(jf[:-5] + ".wav", ref, {"granularity": "word"})
    o, n = old["result"], new
    print("=== %s  ref=%s" % (jf.split("/")[-1], ref))
    print("    句分 %.4f → %.4f   bad %d → %d" %
          (o["sentence_score"], n["sentence_score"], o["bad_phone_count"], n["bad_phone_count"]))
    for ow, nw in zip(o["words"], n["words"]):
        heard = "".join("-%s" % h for h in nw.get("heard", []))[1:]
        if ow["pron_status"] != nw["pron_status"] or abs(ow["score"] - nw["score"]) > 1e-4:
            print("    %-12s %.2f %-9s → %.2f %-9s  听到:%s" %
                  (ow["ref"], ow["score"], ow["pron_status"], nw["score"], nw["pron_status"], heard))
        else:
            print("    %-12s %.2f %-9s (不变)            听到:%s" %
                  (ow["ref"], ow["score"], ow["pron_status"], heard))
        for op, np_ in zip(ow.get("phones", []), nw.get("phones", [])):
            if op["pron_status"] != np_["pron_status"]:
                changed += 1
                print("        %-4s %-9s → %-9s  peak=%s(%s)  %s" %
                      (op["ph"], op["pron_status"], np_["pron_status"],
                       np_.get("peak_t"), np_.get("peak_raw"), np_.get("hint", "")))
print("\n共 %d 个音素改判" % changed)
