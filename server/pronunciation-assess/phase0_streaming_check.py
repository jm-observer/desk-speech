"""Phase 0 / 0.5:流式声学可行性验证(实时评测方案④的风险 de-risk)。

问题:wav2vec2 是**双向全上下文**模型,流式只能给它**有限左上下文 + 少量右 lookahead**。
分块喂入后边界帧后验会失真多少?延迟随句长涨多少?

两种模式:
  --mode accuracy (默认):整段 oracle vs 分块流式,比逐帧 top-1/top-3 + KL + 逐音素 GOP 翻判
                          (oracle 对齐 与 流式对齐两种口径)+ 翻判贴阈值占比 + 对齐漂移。
  --mode latency        :增长前缀(全左上下文)模拟流式,测每块推理 p50/p95/max + 实时率,
                          并给"理想缓存(仅算新帧)"下界作对比。--repeat N 拼接拉长音频。

诚实边界(GPT review 后明确):
  - accuracy 的「oracle 对齐翻判」固定整段对齐、只换后验 → **只验声学层**;「流式对齐翻判」用
    流式后验**重跑离线 forced_align**(全局 Viterbi)→ 更贴近但**仍非真·帧同步在线对齐**,
    后者的抖动留 Phase 1 验。
  - 合成/口音样本能暴露问题但估不准真实翻判率;真人样本请用 --audio 传入。

用法(pronunciation-assess 容器内,GPU):
  python phase0_streaming_check.py <audio> "<ref_text>" [--augment none|noise|echo] [--snr DB]
  python phase0_streaming_check.py <audio> "<ref_text>" --mode latency [--repeat N]
"""
import argparse
import sys
import time

import numpy as np
import torch

import gop


# ----------------------------- 推理 / 音频 -----------------------------

def run_emission(samples: np.ndarray) -> np.ndarray:
    with torch.no_grad():
        inputs = gop._processor(samples, sampling_rate=gop.TARGET_SR, return_tensors="pt")
        iv = inputs.input_values.to(gop.DEVICE)
        logits = gop._model(iv).logits[0]
        return torch.log_softmax(logits, dim=-1).cpu().numpy()


def augment(wav: np.ndarray, kind: str, snr_db: float) -> np.ndarray:
    """样本增广:noise=加高斯白噪到指定 SNR;echo=加延迟衰减回声(模拟外放串扰)。"""
    if kind == "noise":
        sig_p = float(np.mean(wav ** 2)) + 1e-12
        noise_p = sig_p / (10 ** (snr_db / 10))
        rng = np.random.default_rng(0)
        return (wav + rng.normal(0, np.sqrt(noise_p), wav.shape)).astype(np.float32)
    if kind == "echo":
        d = int(0.12 * gop.TARGET_SR)  # 120ms 延迟
        out = wav.copy()
        out[d:] += 0.4 * wav[:-d]      # 衰减 0.4 的回声
        return out.astype(np.float32)
    return wav


def build_stream_emission(wav, full_T, spf, chunk_ms, look_ms, left_ms):
    sr = gop.TARGET_SR
    chunk_s = max(1, int(chunk_ms * sr / 1000))
    look_s = int(look_ms * sr / 1000)
    left_s = int(left_ms * sr / 1000)
    n = len(wav)
    stream = None
    g0 = 0
    while g0 < full_T:
        c_s0 = int(g0 * spf)
        c_s1 = min(n, c_s0 + chunk_s)
        g1 = min(full_T, int(round(c_s1 / spf)))
        if g1 <= g0:
            g1 = min(full_T, g0 + 1)
            c_s1 = min(n, int(g1 * spf))
        w_s0 = max(0, c_s0 - left_s)
        w_s1 = min(n, c_s1 + look_s)
        emit = run_emission(wav[w_s0:w_s1])
        if stream is None:
            stream = np.zeros((full_T, emit.shape[1]), dtype=emit.dtype)
        win_f0 = int(round(w_s0 / spf))
        for g in range(g0, g1):
            lf = g - win_f0
            stream[g] = emit[lf] if 0 <= lf < emit.shape[0] else 0.0
        g0 = g1
    return stream


# ----------------------------- 对齐 / GOP -----------------------------

def ref_target_ids(ref_text):
    from g2p_en import G2p
    g2p = G2p()
    words = gop.tokenize_words(ref_text)
    flat_ph, flat_id, owner = [], [], []
    for wi, w in enumerate(words):
        for ph in gop._g2p_word(w, g2p):
            tid = gop._resolve_token_id(ph)
            if tid is not None:
                flat_ph.append(gop.strip_stress(ph))
                flat_id.append(tid)
                owner.append(wi)
    return words, flat_ph, flat_id, owner


def align_spans(emission, flat_id):
    import torchaudio.functional as F
    em = torch.from_numpy(emission)
    tgt = torch.tensor([flat_id], dtype=torch.int64)
    aligned, scores = F.forced_align(em.unsqueeze(0), tgt, blank=gop._blank_id)
    return [s for s in F.merge_tokens(aligned[0], scores[0]) if s.token != gop._blank_id]


def phone_gop(emission, spans, flat_id, cal):
    out = []
    for i in range(len(flat_id)):
        if i >= len(spans):
            out.append((0.0, gop.PRON_BAD)); continue
        sp = spans[i]
        seg = emission[sp.start:sp.end + 1]
        if seg.shape[0] == 0:
            out.append((0.0, gop.PRON_BAD)); continue
        s01 = gop.calibrate(float(seg[:, flat_id[i]].max()), cal)
        out.append((s01, gop.pron_status(s01, cal)))
    return out


def near_threshold(score, cal, eps=0.08):
    """该分是否贴在某个分档阈值 ±eps 内(翻判易被放大的标志)。"""
    return abs(score - cal.ok_min) < eps or abs(score - cal.warn_min) < eps


# ----------------------------- 指标 -----------------------------

def frame_metrics(full, stream):
    a_full = full.argmax(axis=1)
    top1 = float((a_full == stream.argmax(axis=1)).mean())
    top3_idx = np.argpartition(stream, -3, axis=1)[:, -3:]
    top3 = float(np.mean([a_full[i] in top3_idx[i] for i in range(len(a_full))]))
    p = np.exp(full)
    kl = float((p * (full - stream)).sum(axis=1).clip(min=0).mean())
    return top1, top3, kl


def flip_stats(full_gop, other_gop, cal):
    """逐音素翻判数 + 平均分差 + 翻判中贴阈值占比。"""
    flip = near = 0
    dsum = 0.0
    for (sf, stf), (ss, sts) in zip(full_gop, other_gop):
        dsum += abs(sf - ss)
        if stf != sts:
            flip += 1
            if near_threshold(sf, cal) or near_threshold(ss, cal):
                near += 1
    n = max(1, len(full_gop))
    near_share = (near / flip) if flip else 0.0
    return flip / n, dsum / n, near_share


# ----------------------------- 模式 -----------------------------

def mode_accuracy(wav, ref_text, cal):
    full = run_emission(wav)
    T = full.shape[0]
    spf = len(wav) / T
    print(f"audio={len(wav)/gop.TARGET_SR:.2f}s  frames={T}  spf={spf:.1f}  model={gop.MODEL_ID}")
    words, flat_ph, flat_id, owner = ref_target_ids(ref_text)
    print(f"ref='{ref_text}'  音素={len(flat_id)} {flat_ph}")
    if not flat_id:
        print("无可对齐音素,跳过"); return
    full_spans = align_spans(full, flat_id)
    full_gop = phone_gop(full, full_spans, flat_id, cal)

    configs = [(320, 160, 99999), (320, 160, 1280), (320, 160, 640), (640, 320, 99999)]
    print("\n" + "=" * 104)
    print(f"{'chunk/look/left':<18}{'top1':>7}{'top3':>7}{'KL':>7}"
          f"{'分差':>7}{'翻判(oracle)':>13}{'翻判(流式)':>12}{'翻判贴阈值':>11}{'漂移p50/p95':>13}")
    print("-" * 104)
    for chunk, look, left in configs:
        stream = build_stream_emission(wav, T, spf, chunk, look, left)
        top1, top3, kl = frame_metrics(full, stream)
        # oracle 对齐(只换后验,验声学)
        g_oracle = phone_gop(stream, full_spans, flat_id, cal)
        fr_o, delta, near_o = flip_stats(full_gop, g_oracle, cal)
        # 流式对齐(流式后验重跑 forced_align,更贴近但仍非真在线)
        str_spans = align_spans(stream, flat_id)
        g_str = phone_gop(stream, str_spans, flat_id, cal)
        fr_s, _, near_s = flip_stats(full_gop, g_str, cal)
        shifts = [abs(a.start - b.start) for a, b in zip(full_spans, str_spans)]
        p50 = int(np.percentile(shifts, 50)) if shifts else 0
        p95 = int(np.percentile(shifts, 95)) if shifts else 0
        left_s = "∞" if left >= 99999 else str(left)
        near = max(near_o, near_s)
        print(f"{f'{chunk}/{look}/{left_s}':<18}{top1*100:>6.1f}%{top3*100:>6.1f}%{kl:>7.2f}"
              f"{delta:>7.3f}{fr_o*100:>12.1f}%{fr_s*100:>11.1f}%{near*100:>10.0f}%{f'{p50}/{p95}':>13}")
    print("=" * 104)


def mode_latency(wav, repeat):
    if repeat > 1:
        wav = np.tile(wav, repeat)
    dur = len(wav) / gop.TARGET_SR
    # 先拿整段帧数定 spf。
    full_T = run_emission(wav).shape[0]
    spf = len(wav) / full_T
    chunk_ms, look_ms = 320, 160
    chunk_s = int(chunk_ms * gop.TARGET_SR / 1000)
    look_s = int(look_ms * gop.TARGET_SR / 1000)
    n = len(wav)

    grow_ms, ideal_ms = [], []
    c0 = 0
    while c0 < n:
        c1 = min(n, c0 + chunk_s)
        # 全左上下文(增长前缀):喂 [0, c1+look]
        t = time.perf_counter()
        run_emission(wav[0:min(n, c1 + look_s)])
        grow_ms.append((time.perf_counter() - t) * 1000)
        # 理想缓存下界:只算本块新帧(有界窗 [c0-小左, c1+look])
        t = time.perf_counter()
        run_emission(wav[max(0, c0 - chunk_s):min(n, c1 + look_s)])
        ideal_ms.append((time.perf_counter() - t) * 1000)
        c0 = c1

    def stat(a): return (np.percentile(a, 50), np.percentile(a, 95), np.max(a), np.sum(a))
    g50, g95, gmax, gtot = stat(grow_ms)
    i50, i95, imax, itot = stat(ideal_ms)
    print(f"audio={dur:.1f}s  chunks={len(grow_ms)}  chunk={chunk_ms}ms+{look_ms}ms瞻  device={gop.DEVICE}")
    print(f"{'实现':<22}{'p50(ms)':>9}{'p95(ms)':>9}{'max(ms)':>9}{'总(ms)':>9}{'实时率(总/音频)':>16}")
    print(f"{'增长前缀(全左,O(n²))':<22}{g50:>9.1f}{g95:>9.1f}{gmax:>9.1f}{gtot:>9.0f}{gtot/(dur*1000):>15.2f}x")
    print(f"{'理想缓存(有界窗,下界)':<22}{i50:>9.1f}{i95:>9.1f}{imax:>9.1f}{itot:>9.0f}{itot/(dur*1000):>15.2f}x")
    print("注:增长前缀末块 p95/max 越接近整段耗时→长句必须上状态缓存;实时率<1 才跟得上嘴。")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("audio"); ap.add_argument("ref_text", nargs="?", default="")
    ap.add_argument("--mode", choices=["accuracy", "latency"], default="accuracy")
    ap.add_argument("--augment", choices=["none", "noise", "echo"], default="none")
    ap.add_argument("--snr", type=float, default=15.0)
    ap.add_argument("--repeat", type=int, default=1)
    args = ap.parse_args()

    gop._load_model()
    cal = gop.Calibration.load(gop.CALIBRATION_PATH)
    wav = gop._load_audio_16k(args.audio).numpy()
    if args.augment != "none":
        wav = augment(wav, args.augment, args.snr)
        print(f"[augment={args.augment} snr={args.snr}dB]")

    if args.mode == "latency":
        mode_latency(wav, args.repeat)
    else:
        mode_accuracy(wav, args.ref_text, cal)


if __name__ == "__main__":
    sys.exit(main())
