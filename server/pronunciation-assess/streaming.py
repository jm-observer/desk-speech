"""流式发音评测会话(Phase 2:把 Phase 0/1 的 spike 逻辑产品化)。

`StreamingAssessor`:边收音频块边出**临时**逐词/音素分(committed→partial),整句结束用**批量
GOP**(`gop.assess`)出**权威分**(final)。设计见 toolkit `docs/english-shadow-realtime-design.md`
§4(批量 GOP 作 finalizer)/§6(WS 契约)。

实现取舍(对齐 Phase 0/0.5/1 实测):
  - **全/长左上下文 + 小右瞻**:每次 push 在「累积全缓冲」上重算后验(短句 O(n²) 实测够快,
    p95<100ms@20s)。左上下文不可砍(砍则崩)。
  - **真·在线 CTC Viterbi**:逐帧推进 + 回溯;音素离开前沿 `commit` 帧后落定,emit 一次。
  - **committed→final 稳、但 commit 前 live 会抖**(错读处),故只在「落定」时 emit partial,
    commit 前的 live 抖动交由前端 tentative 渲染(本模块不发未落定分)。

推理是同步 torch;WS handler 用 run_in_executor + 信号量包 `push`/`finish`(见 app.py),
避免阻塞事件循环 + 串行化 GPU。
"""
from __future__ import annotations

import os
import tempfile
import wave

import numpy as np

import gop


def _run_emission(samples: np.ndarray) -> np.ndarray:
    import torch
    with torch.no_grad():
        inputs = gop._processor(samples, sampling_rate=gop.TARGET_SR, return_tensors="pt")
        iv = inputs.input_values.to(gop.DEVICE)
        logits = gop._model(iv).logits[0]
        return torch.log_softmax(logits, dim=-1).cpu().numpy()


def _build_ctc_graph(flat_id, blank):
    L = len(flat_id)
    S = 2 * L + 1
    labels = [blank] * S
    phone_of = [-1] * S
    for k in range(L):
        labels[2 * k + 1] = flat_id[k]
        phone_of[2 * k + 1] = k
    prev_of = []
    for s in range(S):
        ps = [s]
        if s - 1 >= 0:
            ps.append(s - 1)
        if s - 2 >= 0 and s % 2 == 1 and flat_id[(s - 1) // 2] != flat_id[(s - 3) // 2]:
            ps.append(s - 2)
        prev_of.append(ps)
    return labels, phone_of, prev_of


def _online_viterbi(emission, flat_id, blank):
    """整段在线 Viterbi(每次 push 在累积缓冲上重跑;短句够快)。返回 backptr/best_at/labels/phone_of/dp_last。"""
    labels, phone_of, prev_of = _build_ctc_graph(flat_id, blank)
    T, S = emission.shape[0], len(labels)
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
    return backptr, best_at, labels, phone_of, dp


def _traceback(backptr, phone_of, end_t, end_s):
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


def _peak_gop(emission, span, canon_id, cal):
    seg = emission[span[0]:span[1] + 1]
    if seg.shape[0] == 0:
        return 0.0
    return gop.calibrate(float(seg[:, canon_id].max()), cal)


class StreamingAssessor:
    """一次流式跟读会话。push() 喂 PCM(s16le 16k 单声道)块,返回**新落定**的词级 partial 更新。"""

    def __init__(self, ref_text: str, granularity: str = "word",
                 look_ms: int = 160, commit_frames: int = 4):
        gop._load_model()
        self.cal = gop.Calibration.load(gop.CALIBRATION_PATH)
        self.ref_text = ref_text
        self.granularity = granularity
        self.look_s = int(look_ms * gop.TARGET_SR / 1000)
        self.commit_frames = commit_frames
        self.words, self.flat_ph, self.flat_id, self.owner = _ref_targets(ref_text)
        self.buf = np.zeros(0, dtype=np.float32)
        self.committed: dict[int, tuple] = {}   # ph -> (score, status, span)
        self.emitted_words: set[int] = set()

    def push(self, pcm_bytes: bytes) -> list[dict]:
        """喂一块 PCM,返回本次**新落定**的词 partial 事件列表(词的全部音素都落定才 emit)。"""
        chunk = np.frombuffer(pcm_bytes, dtype=np.int16).astype(np.float32) / 32768.0
        self.buf = np.concatenate([self.buf, chunk])
        if not self.flat_id or len(self.buf) < gop.TARGET_SR * 0.2:
            return []
        emission = _run_emission(self.buf)
        T = emission.shape[0]
        backptr, best_at, labels, phone_of, _ = _online_viterbi(emission, self.flat_id, gop._blank_id)
        # 当前前沿音素(末帧最佳态)。
        cur_ph = phone_of[best_at[T - 1]]
        if cur_ph < 0 and T >= 2:
            cur_ph = phone_of[best_at[T - 2]]
        sp_now = _traceback(backptr, phone_of, T - 1, best_at[T - 1])
        # 新落定:< 前沿 且 前沿已离开 commit 帧 且 未落定。
        for ph, (s0, s1) in sp_now.items():
            if ph in self.committed or cur_ph < 0 or ph >= cur_ph:
                continue
            if T - 1 - s1 >= self.commit_frames:
                sc = _peak_gop(emission, (s0, s1), self.flat_id[ph], self.cal)
                self.committed[ph] = (sc, gop.pron_status(sc, self.cal), (s0, s1))
        return self._roll_up_words()

    def _roll_up_words(self) -> list[dict]:
        """某词的全部(已登记)音素都落定 → emit 一次该词 partial。"""
        out = []
        for wi, w in enumerate(self.words):
            if wi in self.emitted_words:
                continue
            phs = [i for i, o in enumerate(self.owner) if o == wi]
            if not phs or any(i not in self.committed for i in phs):
                continue
            scores = [self.committed[i][0] for i in phs]
            wscore = gop.aggregate_word(scores)
            ev = {"word_index": wi, "ref": w, "score": round(wscore, 4),
                  "pron_status": gop.pron_status(wscore, self.cal), "final": False}
            if self.granularity != "sentence":
                ev["phones"] = [
                    {"ph": self.flat_ph[i], "score": round(self.committed[i][0], 4),
                     "pron_status": self.committed[i][1]} for i in phs
                ]
            out.append(ev)
            self.emitted_words.add(wi)
        return out

    def finish(self) -> dict:
        """整句结束:把累积缓冲落临时 wav,调**批量 GOP**(gop.assess)出权威分。"""
        if len(self.buf) == 0:
            return {"ref_text": self.ref_text, "sentence_score": 0.0, "words": [],
                    "bad_phone_count": 0, "model": gop.MODEL_ID}
        fd, path = tempfile.mkstemp(suffix=".wav", prefix="stream-")
        os.close(fd)
        try:
            pcm = (np.clip(self.buf, -1, 1) * 32767).astype(np.int16)
            with wave.open(path, "wb") as wf:
                wf.setnchannels(1)
                wf.setsampwidth(2)
                wf.setframerate(gop.TARGET_SR)
                wf.writeframes(pcm.tobytes())
            return gop.assess(path, self.ref_text, {"granularity": self.granularity})
        finally:
            try:
                os.remove(path)
            except OSError:
                pass


def _ref_targets(ref_text):
    """ref_text → (展示词, 逐音素 ARPAbet, 对齐目标 vocab id, 每音素归属词序)。复用 gop 解析。"""
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
