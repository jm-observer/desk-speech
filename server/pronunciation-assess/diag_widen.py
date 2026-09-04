"""实验:CTC 尖峰对齐的 span 只有 1~2 帧,音素的真实声学证据大多落在 span 之外的 blank 帧里
——现在的 GOP 只看那 1~2 帧,把慢读/连读时"读了但尖峰挪了位"的音一律判成错读。

本脚本比较两种打分窗口,量化「扩窗」能救回多少、会不会放过真错读:
  narrow = 现状(forced_align 的 span 本身)
  wide   = 扩到**相邻音素尖峰之间**(严格不越过邻居,故不会借到别的词的同音素),再封顶 ±MAX 帧

**结论(2026-08-31,已验证:此路不通,别再试一遍)**:对 3 条真实录音 33 个音素跑下来,
窄窗判 bad 的 20 个音素**一个都没被扩窗救回**。原因是证据压根不在邻域里——用户没发出
那个音时,扩到 240ms 窗内 canonical 后验依旧是地板值。真正该做的是分清「整段都没有这个
音(吞音,判错)」与「音在别处(对齐错位,存疑)」,即 gop.py 的 `not_heard_anywhere`。
脚本保留作反证:再有人怀疑"是不是窗口太窄冤枉了用户",跑一遍就知道不是。

用法(容器内):
  python3 /app/diag_widen.py /debug/assess-*.json      # 按 dump 批量
  python3 /app/diag_widen.py /debug/xxx.wav "ref text"  # 单条
"""
import glob
import json
import sys
import wave

import numpy as np
import torch
import torchaudio
from g2p_en import G2p

import gop

# 扩窗封顶:一个音素再慢也不会拖过这么多帧(20ms/帧 → 300ms)。防词间长停顿把窗撑爆。
MAX_PAD_FRAMES = 15

gop._load_model()
_g2p = G2p()
CAL = gop.Calibration.load(None)


def targets(ref):
    fp, fraw, fid, fids, owner = [], [], [], [], []
    words = gop.tokenize_words(ref)
    for wi, w in enumerate(words):
        for ph in gop._g2p_word(w, _g2p):
            ids = gop._resolve_token_ids(ph)
            if ids:
                fp.append(gop.strip_stress(ph)); fraw.append(ph)
                fid.append(ids[0]); fids.append(ids); owner.append(wi)
    return words, fp, fraw, fid, fids, owner


def score_window(em, lo, hi, canon_ids, excl_ids, blank):
    """在 [lo,hi] 帧窗内算 GOP(与 assess 同口径:canonical 峰 - 竞争惩罚)。"""
    seg = em[lo:hi + 1]
    if seg.shape[0] == 0:
        return None, None, None
    gop_raw = max(float(seg[:, c].max()) for c in canon_ids)
    comp = seg.max(axis=0).copy()
    for c in excl_ids:
        if 0 <= c < comp.shape[0]:
            comp[c] = -1e30
    if 0 <= blank < comp.shape[0]:
        comp[blank] = -1e30
    competitor = float(comp.max())
    eff = gop_raw - max(0.0, competitor - gop_raw)
    return gop_raw, competitor, eff


def analyze(wavf, ref):
    with wave.open(wavf, "rb") as w:
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0
    wav = torch.from_numpy(pcm)
    with torch.no_grad():
        iv = gop._processor(wav.numpy(), sampling_rate=16000, return_tensors="pt").input_values.to(gop.DEVICE)
        em_t = torch.log_softmax(gop._model(iv).logits[0], dim=-1).cpu()
    em = em_t.numpy()
    words, fp, fraw, fid, fids, owner = targets(ref)
    al, sc = torchaudio.functional.forced_align(
        em_t.unsqueeze(0), torch.tensor([fid]), blank=gop._blank_id)
    spans = [s for s in torchaudio.functional.merge_tokens(al[0], sc[0], blank=gop._blank_id)
             if s.token != gop._blank_id]
    if len(spans) != len(fid):
        print("  !! spans 与目标不一致,跳过"); return []
    T = em.shape[0]
    frame_sec = (wav.shape[0] / T) / 16000

    rows = []
    print("  %-4s %-10s %-16s %-16s %s" % ("ph", "词", "narrow(现状)", "wide(扩窗)", "变化"))
    for i, ph in enumerate(fp):
        sp = spans[i]
        canon = fids[i]
        excl = set(canon)
        if i > 0:
            excl.update(fids[i - 1])
        if i + 1 < len(fids):
            excl.update(fids[i + 1])
        # 窄窗 = 现状
        n_raw, _, n_eff = score_window(em, sp.start, sp.end, canon, excl, gop._blank_id)
        # 宽窗 = 严格夹在相邻音素尖峰之间(不越邻居),再按 MAX_PAD 封顶
        lo = (spans[i - 1].end + 1) if i > 0 else 0
        hi = (spans[i + 1].start - 1) if i + 1 < len(spans) else T - 1
        lo = max(lo, sp.start - MAX_PAD_FRAMES)
        hi = min(hi, sp.end + MAX_PAD_FRAMES)
        lo = min(lo, sp.start); hi = max(hi, sp.end)
        w_raw, _, w_eff = score_window(em, lo, hi, canon, excl, gop._blank_id)
        n_s = gop.calibrate(n_eff, CAL); w_s = gop.calibrate(max(n_eff, w_eff), CAL)
        n_st = gop.pron_status(n_s, CAL); w_st = gop.pron_status(w_s, CAL)
        mark = "" if n_st == w_st else ("  ← 救回 %s→%s" % (n_st, w_st))
        print("  %-4s %-10s %.2f %-9s   %.2f %-9s %s" %
              (ph, words[owner[i]], n_s, n_st, w_s, w_st, mark))
        rows.append(dict(ph=ph, word=words[owner[i]], n=n_s, w=w_s, n_st=n_st, w_st=w_st,
                         frames_narrow=sp.end - sp.start + 1, frames_wide=hi - lo + 1,
                         frame_sec=frame_sec))
    return rows


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0].endswith(".wav"):
        pairs = [(args[0], args[1])]
    else:
        pats = args or ["/debug/assess-*.json"]
        files = sorted({f for p in pats for f in glob.glob(p)})
        pairs = []
        for jf in files:
            if not jf.endswith(".json"):
                continue
            try:
                d = json.load(open(jf))
                pairs.append((jf[:-5] + ".wav", d["ref_text"]))
            except Exception:  # noqa: BLE001
                continue
    allrows = []
    for wavf, ref in pairs:
        print("=== %s  ref=%s" % (wavf.split("/")[-1], ref))
        allrows += analyze(wavf, ref)
    saved = [r for r in allrows if r["n_st"] == "bad" and r["w_st"] != "bad"]
    print("\n合计 %d 个音素,窄窗判 bad %d 个,扩窗救回 %d 个" %
          (len(allrows), sum(1 for r in allrows if r["n_st"] == "bad"), len(saved)))
    from collections import Counter
    print("救回分布:", Counter("%s/%s" % (r["word"], r["ph"]) for r in saved).most_common())
    if allrows:
        print("窗宽中位数: narrow %.0f 帧 / wide %.0f 帧" %
              (np.median([r["frames_narrow"] for r in allrows]),
               np.median([r["frames_wide"] for r in allrows])))


main()
