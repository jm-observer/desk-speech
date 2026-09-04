"""A/B:同一段音频、同一进程内,只切换一条判据,看它到底改了什么。

回归脚本每次重新合成 TTS,音频不同 → 单次结果差异分不清是"规则改了"还是"音频抖了"。
本脚本对**同一个 wav** 跑两遍 assess(新判据开 / 关),差异必然只来自规则本身。

用法(容器内):python3 /app/diag_ab.py /debug/x.wav "ref text" [更多 wav ref ...]
"""
import sys

import gop

pairs = list(zip(sys.argv[1::2], sys.argv[2::2]))
for wavf, ref in pairs:
    print("=== %s  ref=%s" % (wavf.split("/")[-1], ref))
    gop.NOT_HEARD_PEAK_RAW = -1e9          # 关:回到"段内没证据就算没对齐好"的旧兜底
    off = gop.assess(wavf, ref, {"granularity": "word"})
    gop.NOT_HEARD_PEAK_RAW = -3.0          # 开
    on = gop.assess(wavf, ref, {"granularity": "word"})
    print("    句分 %.4f → %.4f   bad %d → %d" %
          (off["sentence_score"], on["sentence_score"],
           off["bad_phone_count"], on["bad_phone_count"]))
    for ow, nw in zip(off["words"], on["words"]):
        diffs = [(op, np_) for op, np_ in zip(ow.get("phones", []), nw.get("phones", []))
                 if op["pron_status"] != np_["pron_status"]]
        tag = "" if not diffs else "  ← 有改判"
        print("    %-10s %.2f %-9s → %.2f %-9s  听到:%s%s" %
              (ow["ref"], ow["score"], ow["pron_status"], nw["score"], nw["pron_status"],
               "-".join(nw.get("heard", [])), tag))
        for op, np_ in diffs:
            print("        %-4s %-9s → %-9s  gop=%s peak=%s(%s)  %s" %
                  (op["ph"], op["pron_status"], np_["pron_status"], np_.get("gop_raw"),
                   np_.get("peak_t"), np_.get("peak_raw"), np_.get("hint", "")))
