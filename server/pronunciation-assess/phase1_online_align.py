"""Phase 1:在线 CTC 强制对齐原型 + 抖动/跳变度量(实时评测方案④的对齐风险 de-risk)。

Phase 0/0.5 只验了**声学后验**,对齐用的是 oracle 或流式后验上的**离线** forced_align。本脚本补上
**真·帧同步在线 Viterbi**:对期望音素的 CTC 对齐图逐帧推进 + 回溯,边读边发**临时**逐音素 GOP,
音素离开 Viterbi 前沿后**落定(finalize)**。离线用录好的 wav 模拟分块喂入,不碰传输。

CTC 对齐图(标准):L 个音素 → 状态序列 [blank, p0, blank, p1, ..., pL-1, blank](S=2L+1)。
转移:自环 s→s;前进 s-1→s;跳过中间 blank s-2→s(仅当 s 为音素态且与 s-2 音素不同)。
逐帧 Viterbi:dp[s] = max(allowed prev) + logP(label[s] | frame_t);记 backptr,可任意帧回溯。

度量(回答 GPT 的"在线对齐抖动未验"):
  1. **在线-final vs 批量** 逐音素翻判(在线对齐 run 到底 vs 批量 oracle)——对齐正确性。
  2. **临时→final 分跳变**:每个音素「首次发出的临时分」与「落定分」的差 + 翻判——用户看到的"跳一下"。
  3. **对齐边界抖动**:临时 span 起点 vs final 起点的漂移 p50/p95(帧)。
  4. **finalize 延迟**:音素结束到其分落定隔多少帧。

用法(容器内,GPU):python phase1_online_align.py <audio> "<ref_text>" [--left MS] [--look MS] [--commit N]
"""
import argparse
import sys

import numpy as np

import gop
import phase0_streaming_check as p0


def build_ctc_graph(flat_id, blank):
    """期望音素 → CTC 对齐图。返回 (labels[S], phone_of[S], prev_of[S]=每状态的合法前驱列表)。"""
    L = len(flat_id)
    S = 2 * L + 1
    labels = [blank] * S
    phone_of = [-1] * S
    for k in range(L):
        labels[2 * k + 1] = flat_id[k]
        phone_of[2 * k + 1] = k
    prev_of = []
    for s in range(S):
        ps = [s]                       # 自环
        if s - 1 >= 0:
            ps.append(s - 1)           # 前进
        # 跳过中间 blank:s 为音素态,且与 s-2 的音素不同
        if s - 2 >= 0 and s % 2 == 1 and flat_id[(s - 1) // 2] != flat_id[(s - 3) // 2]:
            ps.append(s - 2)
        prev_of.append(ps)
    return labels, phone_of, prev_of


def online_viterbi(emission, flat_id, blank):
    """逐帧在线 Viterbi。返回 (backptr[T][S], best_state_at[T], labels, phone_of)。"""
    labels, phone_of, prev_of = build_ctc_graph(flat_id, blank)
    T = emission.shape[0]
    S = len(labels)
    NEG = -1e30
    dp = np.full(S, NEG)
    dp[0] = emission[0, labels[0]]
    if S > 1:
        dp[1] = emission[0, labels[1]]
    backptr = np.full((T, S), -1, dtype=np.int32)
    best_at = [int(np.argmax(dp))]
    for t in range(1, T):
        ndp = np.full(S, NEG)
        for s in range(S):
            bp, bv = -1, NEG
            for pv in prev_of[s]:
                if dp[pv] > bv:
                    bv, bp = dp[pv], pv
            ndp[s] = bv + emission[t, labels[s]]
            backptr[t, s] = bp
        dp = ndp
        best_at.append(int(np.argmax(dp)))
    return backptr, best_at, labels, phone_of, dp  # dp = 末帧列,供选 CTC 接受终态


def traceback(backptr, phone_of, end_t, end_s):
    """从 (end_t, end_s) 回溯,返回每音素的 [start,end] 帧区间(仅已出现的音素)。"""
    spans = {}
    s = end_s
    for t in range(end_t, -1, -1):
        ph = phone_of[s]
        if ph >= 0:
            if ph in spans:
                spans[ph][0] = t
            else:
                spans[ph] = [t, t]
        s = backptr[t, s] if t > 0 else s
        if s < 0:
            break
    return {k: (v[0], v[1]) for k, v in spans.items()}


def peak_gop(emission, span, canon_id, cal):
    s0, s1 = span
    seg = emission[s0:s1 + 1]
    if seg.shape[0] == 0:
        return 0.0
    return gop.calibrate(float(seg[:, canon_id].max()), cal)


def run(audio, ref_text, left_ms, look_ms, commit_frames):
    gop._load_model()
    cal = gop.Calibration.load(gop.CALIBRATION_PATH)
    wav = gop._load_audio_16k(audio).numpy()
    full = p0.run_emission(wav)
    T = full.shape[0]
    spf = len(wav) / T
    words, flat_ph, flat_id, owner = p0.ref_target_ids(ref_text)
    print(f"audio={len(wav)/gop.TARGET_SR:.2f}s frames={T} 音素={len(flat_id)} {flat_ph}")
    if not flat_id:
        print("无可对齐音素"); return

    # 批量 oracle(整段 forced_align)逐音素分 —— 权威基准。
    batch_spans = p0.align_spans(full, flat_id)
    batch_gop = [(peak_gop(full, (sp.start, sp.end), flat_id[i], cal)) for i, sp in enumerate(batch_spans)]
    batch_status = [gop.pron_status(s, cal) for s in batch_gop]

    # 流式后验(长左 + 小右瞻)—— 在线对齐喂这个。
    stream = p0.build_stream_emission(wav, T, spf, 320, look_ms, left_ms)
    backptr, best_at, labels, phone_of, dp_last = online_viterbi(stream, flat_id, gop._blank_id)
    S = len(labels)
    L = len(flat_id)

    # 逐帧推进:① 记录每音素首次落定(前沿越过 commit 帧)的临时 span + 落定帧;
    #          ② 记录每音素 commit 前的 **live tentative** 分序列(回答"实时临时分抖不抖")。
    prov_span, prov_frame = {}, {}
    live_series = {ph: [] for ph in range(L)}  # commit 前逐帧的 live 临时分
    for t in range(T):
        sp_now = traceback(backptr, phone_of, t, best_at[t])
        cur_ph = phone_of[best_at[t]]
        if cur_ph < 0 and t > 0:
            cur_ph = phone_of[best_at[t - 1]]
        for ph in list(sp_now.keys()):
            if ph in prov_span:
                continue
            s0, s1 = sp_now[ph]
            # live:用「截至当前帧」的开放 span 算分(用户当下看到的)。
            live_series[ph].append(peak_gop(stream, (s0, min(s1, t)), flat_id[ph], cal))
            # 落定:前沿已离开本音素 commit 帧。
            if cur_ph >= 0 and ph < cur_ph and t - s1 >= commit_frames:
                prov_span[ph] = (s0, s1)
                prov_frame[ph] = t

    # final:**从 CTC 接受终态**回溯(末音素态 S-2 或末 blank S-1 里分高者),非末帧任意最佳态。
    final_end_s = (S - 1) if dp_last[S - 1] >= dp_last[S - 2] else (S - 2)
    final_spans = traceback(backptr, phone_of, T - 1, final_end_s)
    missing = [ph for ph in range(L) if ph not in final_spans]
    for ph in range(L):  # 未落定的尾音素用 final 兜底
        if ph not in prov_span and ph in final_spans:
            prov_span[ph] = final_spans[ph]
            prov_frame[ph] = T - 1

    # ---- 度量 ----
    on_flip = on_d = c2f_flip = c2f_d = 0.0
    start_drift, signed_lag, end_drift, live_swing = [], [], [], []
    early_commit = live_flip = 0
    n = 0
    for ph in range(L):
        if ph not in final_spans:
            continue
        n += 1
        cid, fs = flat_id[ph], final_spans[ph]
        fin_score = peak_gop(stream, fs, cid, cal)
        fin_status = gop.pron_status(fin_score, cal)
        on_d += abs(fin_score - batch_gop[ph])
        if fin_status != batch_status[ph]:
            on_flip += 1
        # committed → final
        ps = prov_span.get(ph, fs)
        cs = peak_gop(stream, ps, cid, cal)
        c2f_d += abs(cs - fin_score)
        if gop.pron_status(cs, cal) != fin_status:
            c2f_flip += 1
        start_drift.append(abs(ps[0] - fs[0]))
        end_drift.append(ps[1] - fs[1])                    # signed:>0 晚 commit,<0 早 commit
        lag = prov_frame.get(ph, T - 1) - fs[1]
        signed_lag.append(lag)
        if lag < 0:
            early_commit += 1
        # live tentative(commit 前)抖动 + 是否出现过 live 翻判
        ser = live_series[ph]
        if ser:
            live_swing.append(max(ser) - min(ser))
            sts = {gop.pron_status(x, cal) for x in ser}
            if len(sts) > 1:
                live_flip += 1
    n = max(1, n)

    def pp(a, signed=False):
        if not a:
            return (0, 0)
        lo = int(np.percentile(a, 5)) if signed else int(np.percentile(a, 50))
        return (lo, int(np.percentile(a, 95)))
    ms = spf / gop.TARGET_SR * 1000
    print(f"\n配置: 块320ms 左{left_ms}ms 右瞻{look_ms}ms commit{commit_frames}帧  (1帧≈{ms:.0f}ms)")
    print("-" * 78)
    print(f"在线-final vs 批量    : 翻判 {on_flip/n*100:4.1f}%  分差 {on_d/n:.3f}  "
          f"missing音素 {len(missing)}/{L}")
    print(f"committed → final     : 翻判 {c2f_flip/n*100:4.1f}%  分差 {c2f_d/n:.3f}")
    sl50, sl95 = pp(signed_lag, signed=True)
    print(f"finalize 延迟(signed帧): p5 {sl50}  p95 {sl95}  early-commit {early_commit}/{n}  "
          f"(≈{sl50*ms:.0f}/{sl95*ms:.0f}ms)")
    ed50, ed95 = pp(end_drift, signed=True)
    print(f"prov_end - final_end  : p5 {ed50}  p95 {ed95} 帧")
    sd50, sd95 = pp(start_drift)
    print(f"对齐起点抖动(帧)      : p50 {sd50}  p95 {sd95}")
    if live_swing:
        lsw50 = float(np.percentile(live_swing, 50))
        lsw95 = float(np.percentile(live_swing, 95))
        print(f"live tentative 抖动   : 分摆动 p50/p95 {lsw50:.2f}/{lsw95:.2f}  "
              f"出现过 live 翻判的音素 {live_flip}/{n}")
    else:
        print("live tentative 抖动   : 无数据")
    print("-" * 78)
    print("判据:在线-final≈批量+missing低→对齐正确完整;committed→final 跳变低→落定稳;"
          "early-commit 低→未抢跑;live 抖动是 commit 前实时分的真实跳动(UX 按 tentative 渲染)。")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("audio"); ap.add_argument("ref_text")
    ap.add_argument("--left", type=int, default=99999)
    ap.add_argument("--look", type=int, default=160)
    ap.add_argument("--commit", type=int, default=6, help="前沿越过音素多少帧后落定")
    args = ap.parse_args()
    run(args.audio, args.ref_text, args.left, args.look, args.commit)


if __name__ == "__main__":
    sys.exit(main())
