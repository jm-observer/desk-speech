"""英语发音评测(GOP)引擎。

把参考文本经 **G2P** 展开为期望音素序列(ARPAbet),对用户录音用 **wav2vec2 CTC 音素后验**
做**强制对齐**,逐音素算 **GOP**(Goodness of Pronunciation),标定到 0~1,再自底向上聚合到
词、句。对外只暴露 `assess(audio_path, ref_text, opts) -> dict`(契约见
`docs/pronunciation-assess-api.md`,与 toolkit `english-shadow-gop-design.md` §4 对齐)。

概念背景见 `docs/pronunciation-coach-overview.md`:通用 ASR/Whisper 会把不标准发音"脑补"成
正确文本,正好抹掉要检测的错误;故核心是发音评测(音素后验 + 对齐 + GOP),不是识别。

## 分层(重要,便于无 GPU 单测)

- **纯函数区**(本文件上半部,**不导入 torch/transformers/g2p_en**):分词、标定曲线、
  状态分档、词/句聚合、hint 拼接、响应组装。`test_gop.py` 直接 import 这些做断言。
- **重推理区**(下半部,torch/transformers/g2p_en **惰性导入**):模型加载缓存、G2P、
  强制对齐、逐音素 GOP。只在 `assess()` 真正跑评测时触发。

所有分值 **0~1**(已标定),与 toolkit v1 `score`/`threshold` 同区间。
"""
from __future__ import annotations

import math
import os
import re
import json
from dataclasses import dataclass, field
from typing import Optional

# ======================================================================================
# 纯函数区 —— 无重依赖,可被 test_gop.py 直接 import。
# ======================================================================================

# 发音四档(与契约 / toolkit pron_status 对齐)。
PRON_OK = "ok"
PRON_WARN = "warn"
PRON_BAD = "bad"
# uncertain:引擎没把该音素对齐好/没听准 → **不判对错**(灰、不计 bad、不拉低词分)。
# 见 docs/english-shadow-scoring-ui-design.md §3。
PRON_UNCERTAIN = "uncertain"

# 可靠性判据(raw log-后验空间;与标定 a≈0.8,b≈-3 配套)。一个被判 bad 的音素:
# 若「对齐段里 canonical 后验极低(基本没发出)」**且**「段内也没有明确的替代音」→ 多半没对齐上 → uncertain。
# 若有明确替代音(如 /θ/ 段里 /s/ 很强)→ 真替换错读,仍 bad。
UNCERTAIN_CANON_FLOOR = -5.0   # canonical 在对齐段的峰值后验低于此 = 基本没在这儿发出
CONFIDENT_OTHER_RAW = -1.5     # 段内最强竞争音素峰值高于此 = 有明确替代音(真错读)


@dataclass
class Calibration:
    """GOP 原始分(log 后验差,(-inf,0],0=完美)→ 0~1 的标定 + 状态分档阈值。

    标定曲线:`score01 = sigmoid(a * (gop_raw - b))`。`a` 控制陡度,`b` 是 0.5 分对应的
    GOP(中点)。**应由 speechocean762 拟合**(见 README「标定」),此处默认值仅为可跑的
    合理占位,GB10 上线前须用真实数据重标。
    """

    a: float = 4.0
    b: float = -1.0
    # 状态分档:score>=ok_min→ok;>=warn_min→warn;else bad。
    ok_min: float = 0.70
    warn_min: float = 0.45

    @staticmethod
    def load(path: Optional[str]) -> "Calibration":
        """从 JSON 文件加载标定参数;路径为空/不存在/损坏 → 默认值(不致命)。"""
        if not path or not os.path.exists(path):
            return Calibration()
        try:
            with open(path, "r", encoding="utf-8") as f:
                d = json.load(f)
            return Calibration(
                a=float(d.get("a", 4.0)),
                b=float(d.get("b", -1.0)),
                ok_min=float(d.get("ok_min", 0.70)),
                warn_min=float(d.get("warn_min", 0.45)),
            )
        except (ValueError, OSError, json.JSONDecodeError):
            return Calibration()


def calibrate(gop_raw: float, cal: Calibration) -> float:
    """GOP 原始分 → 0~1。数值钳到 [0,1]。"""
    z = cal.a * (gop_raw - cal.b)
    # 防 exp 溢出。
    if z >= 0:
        s = 1.0 / (1.0 + math.exp(-z))
    else:
        e = math.exp(z)
        s = e / (1.0 + e)
    return max(0.0, min(1.0, s))


def pron_status(score01: float, cal: Calibration) -> str:
    """0~1 发音分 → 三档 ok/warn/bad。"""
    if score01 >= cal.ok_min:
        return PRON_OK
    if score01 >= cal.warn_min:
        return PRON_WARN
    return PRON_BAD


def tokenize_words(text: str) -> list[str]:
    """切「展示词」:非字母数字一律作分隔,保留原始词形(与 toolkit Rust 端 normalize 同源)。"""
    return [w for w in re.split(r"[^0-9A-Za-z']+", text) if w]


def strip_stress(arpabet: str) -> str:
    """剥 ARPAbet 重音数字并大写:`IH1` → `IH`、`ay0` → `AY`。"""
    return re.sub(r"\d+$", "", arpabet).upper()


def aggregate_word(phone_scores: list[float]) -> float:
    """音素分 → 词分。均值与最低分加权:一个严重错读音素应明显拉低词分。"""
    if not phone_scores:
        return 0.0
    mean = sum(phone_scores) / len(phone_scores)
    lo = min(phone_scores)
    return round(0.6 * mean + 0.4 * lo, 4)


def aggregate_sentence(word_scores: list[float]) -> float:
    """词分 → 句分。同样均值 + 最低分加权,但句级最低分权重略低。"""
    if not word_scores:
        return 0.0
    mean = sum(word_scores) / len(word_scores)
    lo = min(word_scores)
    return round(0.7 * mean + 0.3 * lo, 4)


# ARPAbet → IPA(仅用于把 hint 拼成「/θ/ 读成了 /s/」这类人类可读文案;评测本身用 ARPAbet)。
_ARPABET_IPA = {
    "AA": "ɑ", "AE": "æ", "AH": "ʌ", "AO": "ɔ", "AW": "aʊ", "AY": "aɪ",
    "B": "b", "CH": "tʃ", "D": "d", "DH": "ð", "EH": "ɛ", "ER": "ɝ",
    "EY": "eɪ", "F": "f", "G": "ɡ", "HH": "h", "IH": "ɪ", "IY": "i",
    "JH": "dʒ", "K": "k", "L": "l", "M": "m", "N": "n", "NG": "ŋ",
    "OW": "oʊ", "OY": "ɔɪ", "P": "p", "R": "ɹ", "S": "s", "SH": "ʃ",
    "T": "t", "TH": "θ", "UH": "ʊ", "UW": "u", "V": "v", "W": "w",
    "Y": "j", "Z": "z", "ZH": "ʒ",
}


def arpabet_to_ipa(ph: str) -> str:
    """ARPAbet → IPA 符号(标准型,供 hint 文案);未知音素回退原样。"""
    return _ARPABET_IPA.get(strip_stress(ph), ph)


# 某些 ARPAbet 音素在不同模型 vocab 里有多种 IPA 写法;给候选列表逐个试。
# 典型:AH 在 espeak/IPA 模型里常并入 schwa ə;G 有 IPA ɡ(U+0261) 与 ASCII g 两种;
# ER 有 ɝ/ɚ;R 有 ɹ/r;AO 在 schwa-merged 模型里可能没有独立 token(退 ɑ)。
_ARPABET_IPA_ALTS = {
    "AH": ["ʌ", "ə"],
    "G": ["ɡ", "g"],
    "ER": ["ɝ", "ɚ", "ə"],
    "R": ["ɹ", "r"],
    "AO": ["ɔ", "ɑ"],
    "UH": ["ʊ", "u"],
}


def model_token_candidates(ph: str) -> list[str]:
    """一个 ARPAbet 音素 → 在声学模型 vocab 里可能的 token 写法,按优先级排。

    覆盖**两类模型**:ARPAbet 输出(直接用大写去重音形)与 IPA/espeak 输出(用 IPA 写法 +
    备选)。`assess()` 拿这串候选逐个查 vocab,首个命中即该音素的对齐目标 id。纯函数,可单测。
    """
    base = strip_stress(ph)
    cands = [base]  # ARPAbet 模型:vocab 直接是大写 ARPAbet
    # 弱读 schwa:有些模型(如 L2 phoneme)把 AH 的弱读形输出成独立的 ax,G2P 仍标 AH → 一并接受。
    if base == "AH":
        cands.append("AX")
    primary = _ARPABET_IPA.get(base)
    if primary and primary not in cands:
        cands.append(primary)
    for alt in _ARPABET_IPA_ALTS.get(base, []):
        if alt not in cands:
            cands.append(alt)
    return cands


# IPA → ARPAbet 反查(把模型输出的「实际音素」转回 ARPAbet,供 actual_ph / hint)。
_IPA_ARPABET = {ipa: arp for arp, ipa in _ARPABET_IPA.items()}
for _arp, _alts in _ARPABET_IPA_ALTS.items():
    for _ipa in _alts:
        _IPA_ARPABET.setdefault(_ipa, _arp)


def model_token_to_arpabet(tok: str) -> str:
    """模型 vocab token(IPA 或 ARPAbet)→ ARPAbet。

    IPA 模型:查 IPA→ARPAbet 反表;ARPAbet 模型:去重音大写即可;查不到回退原样。
    """
    if tok in _IPA_ARPABET:
        return _IPA_ARPABET[tok]
    su = strip_stress(tok)
    if su in _ARPABET_IPA:  # 本就是 ARPAbet
        return su
    return tok


def build_hint(expected_ph: str, actual_ph: Optional[str]) -> str:
    """据「期望 vs 实际」音素拼人类可读纠音文案。`actual` 为空(漏读/无替代)时给通用提示。"""
    exp_ipa = arpabet_to_ipa(expected_ph)
    if actual_ph and strip_stress(actual_ph) != strip_stress(expected_ph):
        return f"/{exp_ipa}/ 读成了 /{arpabet_to_ipa(actual_ph)}/"
    return f"/{exp_ipa}/ 发音偏弱"


@dataclass
class PhoneEval:
    """逐音素评测的中间结果(纯数据,便于组装与测试)。"""

    ph: str
    score: float
    status: str
    expected_ph: Optional[str] = None
    actual_ph: Optional[str] = None
    hint: Optional[str] = None
    # 对齐可靠性 + 时间段(供 UI 明细表 / 区分"读错"vs"没对齐")。
    reliable: bool = True
    t_start: Optional[float] = None
    t_end: Optional[float] = None
    # 诊断:canonical 在整段的**全局峰时间**(秒)。落在 [t_start,t_end] 外 = 对齐错位。
    peak_t: Optional[float] = None
    # 诊断:对齐段内 canonical 的峰值 log 后验(原始 GOP,≤0,越接近 0 证据越强)。
    gop_raw: Optional[float] = None


@dataclass
class WordEval:
    ref: str
    score: float
    status: str
    phones: list[PhoneEval] = field(default_factory=list)


def count_bad_phones(words: list[WordEval]) -> int:
    """统计 status==bad 的音素总数(供 passed 判定)。**uncertain 不计**(没对齐上的不算读错)。"""
    return sum(1 for w in words for p in w.phones if p.status == PRON_BAD)


def assemble_response(
    ref_text: str,
    words: list[WordEval],
    transcript: Optional[str],
    model_id: str,
    granularity: str,
) -> dict:
    """把逐词/逐音素评测组装成 `/assess` 契约 JSON(见 docs/pronunciation-assess-api.md)。

    **始终返回 `phones[]`**:评分明细表(逐音素诊断)在整句 / 逐词模式下都要展示,故不再按
    granularity 裁剪(一句几十个音素,带宽可忽略)。`granularity` 现仅保留兼容,不影响内容。
    句级 `sentence_score` 由词分聚合;`bad_phone_count` 始终据音素算。
    """
    _ = granularity  # 不再据此裁剪 phones(明细表整句也需要)
    # 句分只聚合「非 uncertain」的词(没对齐上的词不拉低句分)。
    sentence_score = aggregate_sentence([w.score for w in words if w.status != PRON_UNCERTAIN])
    bad_count = count_bad_phones(words)
    include_phones = True

    out_words = []
    for w in words:
        wd = {"ref": w.ref, "score": round(w.score, 4), "pron_status": w.status}
        if include_phones and w.phones:
            wd["phones"] = [_phone_to_json(p) for p in w.phones]
        out_words.append(wd)

    resp = {
        "ref_text": ref_text,
        "sentence_score": round(sentence_score, 4),
        "words": out_words,
        "bad_phone_count": bad_count,
        "model": model_id,
    }
    if transcript is not None:
        resp["transcript"] = transcript
    return resp


def _phone_to_json(p: PhoneEval) -> dict:
    d = {"ph": p.ph, "score": round(p.score, 4), "pron_status": p.status}
    if p.expected_ph is not None:
        d["expected_ph"] = p.expected_ph
    if p.actual_ph is not None:
        d["actual_ph"] = p.actual_ph
    if p.hint is not None:
        d["hint"] = p.hint
    if not p.reliable:
        d["reliable"] = False
    if p.t_start is not None:
        d["t_start"] = p.t_start
    if p.t_end is not None:
        d["t_end"] = p.t_end
    if p.peak_t is not None:
        d["peak_t"] = p.peak_t
    if p.gop_raw is not None:
        d["gop_raw"] = p.gop_raw
    return d


# ======================================================================================
# 重推理区 —— torch / transformers / g2p_en 惰性导入,只在真正评测时触发。
# ======================================================================================

# 声学模型:wav2vec2 CTC 音素模型(输出 ARPAbet/类 ARPAbet/IPA token)。可经 env 覆盖。
# 默认 = slplab L2-English phoneme(wav2vec2-large-robust,专训非母语英语,带 *_err 误读 token)。
# 实测远胜通用 TIMIT 版:连读长词(delicious)中段不再整段误杀,th→s 真错读仍抓。小写 ARPAbet +
# 弱读 ax,gop 的多候选桥(含 AH→ax)已适配。见 README「模型」。
MODEL_ID = os.environ.get(
    "GOP_MODEL_ID", "slplab/wav2vec2-large-robust-L2-english-phoneme-recognition"
)
DEVICE = os.environ.get("GOP_DEVICE", "cuda")
TARGET_SR = 16000
CALIBRATION_PATH = os.environ.get("GOP_CALIBRATION")

# 进程内缓存(模型常驻,避免逐请求重载 → 满足 ~1-2s 交互延迟)。
_model = None
_processor = None
_vocab = None        # token(大写 ARPAbet) -> id
_vocab_inv = None    # id -> token
_blank_id = 0


def _load_model():
    """惰性加载并缓存 wav2vec2 CTC 音素模型 + processor + vocab。"""
    global _model, _processor, _vocab, _vocab_inv, _blank_id
    if _model is not None:
        return
    import torch  # noqa: F401
    from transformers import AutoModelForCTC, AutoProcessor

    proc = AutoProcessor.from_pretrained(MODEL_ID)
    model = AutoModelForCTC.from_pretrained(MODEL_ID)
    model.eval()
    if DEVICE == "cuda":
        import torch as _t
        if _t.cuda.is_available():
            model = model.to("cuda")
        else:
            globals()["DEVICE"] = "cpu"

    tok = proc.tokenizer
    raw_vocab = tok.get_vocab()  # token_str -> id
    # vocab 同时按「原样 token」(IPA 模型:θ/ə;ARPAbet 模型:TH)与「去重音大写别名」
    # (ARPAbet 模型容错)建键。**绝不对 IPA token .upper()**(θ→Θ 会破匹配),故原样优先。
    vocab = {}
    inv = {}
    for token_str, idx in raw_vocab.items():
        inv[idx] = token_str
        vocab.setdefault(token_str, idx)
        su = strip_stress(token_str)
        vocab.setdefault(su, idx)
    blank = tok.pad_token_id if tok.pad_token_id is not None else 0

    _processor, _model, _vocab, _vocab_inv, _blank_id = proc, model, vocab, inv, blank


def _resolve_token_id(ph: str) -> Optional[int]:
    """ARPAbet 音素 → 模型 vocab **主** id(用作对齐目标);未登录返回 None。"""
    ids = _resolve_token_ids(ph)
    return ids[0] if ids else None


def _resolve_token_ids(ph: str) -> list:
    """ARPAbet 音素 → 所有可接受的 vocab id(去重,按候选优先级)。
    GOP 打分取这组 id 后验的**最大值**(覆盖 ah/ax 这类同音素多写法),对齐仍用主 id。"""
    ids = []
    for cand in model_token_candidates(ph):
        tid = _vocab.get(cand)
        if tid is not None and tid not in ids:
            ids.append(tid)
    return ids


def _load_audio_16k(path: str):
    """用 ffmpeg 把任意格式(webm/opus/wav/mp4…)解码为 16k 单声道 float32 张量。

    避开 torchaudio.load(2.11 改依赖 torchcodec,且对 webm 不稳);ffmpeg 在 base 镜像自带,
    与 FunASR / audio-cleanup 同惯例。返回 1D torch.FloatTensor。
    """
    import subprocess

    import numpy as np
    import torch

    cmd = [
        "ffmpeg", "-nostdin", "-loglevel", "error", "-i", path,
        "-f", "s16le", "-ac", "1", "-ar", str(TARGET_SR), "-",
    ]
    proc = subprocess.run(cmd, capture_output=True)
    if proc.returncode != 0 or not proc.stdout:
        raise RuntimeError(
            "ffmpeg 解码失败: " + proc.stderr.decode("utf-8", "ignore")[:200]
        )
    pcm = np.frombuffer(proc.stdout, dtype=np.int16).astype(np.float32) / 32768.0
    return torch.from_numpy(pcm.copy())


def _g2p_word(word: str, g2p) -> list[str]:
    """单词 → ARPAbet 音素序列(去重音)。OOV 由 g2p_en 的 seq2seq 兜底。"""
    phones = [strip_stress(p) for p in g2p(word) if p.strip() and re.match(r"[A-Za-z]", p)]
    return phones


def assess(audio_path: str, ref_text: str, opts: dict) -> dict:
    """对一段录音做发音评测,返回 `/assess` 契约 dict。

    `opts`:`{"granularity": "sentence"|"word", "lang": "en"}`。重推理全在此函数内,
    上层 `app.py` 用 run_in_executor + Semaphore(1) 串行化 GPU。
    """
    import torch
    import torchaudio
    from g2p_en import G2p  # noqa: F401  （惰性,触发 nltk 资源加载）

    _load_model()
    cal = Calibration.load(CALIBRATION_PATH)
    granularity = str(opts.get("granularity", "word")).strip().lower()

    # ---- 参考文本 → 逐词期望音素(ARPAbet)----
    g2p = G2p()
    display_words = tokenize_words(ref_text)
    word_phones: list[list[str]] = [_g2p_word(w, g2p) for w in display_words]

    # 展平为对齐目标 + 记录每音素归属词。每个 ARPAbet 音素经多候选解析到模型 vocab id
    # (兼容 IPA / ARPAbet 模型);解析不到的音素(模型无对应 token)降级跳过。
    flat_phones: list[str] = []   # ARPAbet 形,供展示 / hint
    flat_ids: list[int] = []      # 对齐目标:模型 vocab 主 id
    flat_idsets: list[list] = []  # 打分用:同音素可接受的全部 id(取后验最大)
    owner_word: list[int] = []
    for wi, phs in enumerate(word_phones):
        for ph in phs:
            ids = _resolve_token_ids(ph)
            if ids:
                flat_phones.append(strip_stress(ph))
                flat_ids.append(ids[0])
                flat_idsets.append(ids)
                owner_word.append(wi)

    # ---- 录音解码 → 16k 单声道(ffmpeg,格式无关)----
    wav = _load_audio_16k(audio_path)  # 1D float32

    # ---- 音素后验(log-softmax)----
    with torch.no_grad():
        inputs = _processor(
            wav.numpy(), sampling_rate=TARGET_SR, return_tensors="pt"
        )
        input_values = inputs.input_values.to(DEVICE)
        logits = _model(input_values).logits[0]  # (T, C)
        emission = torch.log_softmax(logits, dim=-1).cpu()  # (T, C)

    # 无可对齐音素(整句 G2P 全未登录) → 退化为空词评测。
    if not flat_phones:
        words = [WordEval(ref=w, score=0.0, status=PRON_BAD) for w in display_words]
        return assemble_response(ref_text, words, None, MODEL_ID, granularity)

    target_ids = torch.tensor([flat_ids], dtype=torch.int64)

    # ---- 强制对齐(torchaudio CTC forced_align)----
    aligned, scores = torchaudio.functional.forced_align(
        emission.unsqueeze(0), target_ids, blank=_blank_id
    )
    spans = torchaudio.functional.merge_tokens(aligned[0], scores[0])
    # 合并后的 spans 对应非 blank 目标,顺序与 flat_phones 一致。
    spans = [s for s in spans if s.token != _blank_id]

    # 帧→秒(供时间段);emission T 帧覆盖整段 wav。
    frame_sec = (wav.shape[0] / emission.shape[0]) / TARGET_SR

    # ---- 逐音素 GOP + 对齐可靠性 ----
    phone_evals: list[Optional[PhoneEval]] = []
    for i, ph in enumerate(flat_phones):
        if i >= len(spans):
            phone_evals.append(None)
            continue
        sp = spans[i]
        seg = emission[sp.start:sp.end + 1]  # (n, C)
        if seg.shape[0] == 0:
            phone_evals.append(None)
            continue
        canon_ids = flat_idsets[i]
        # CTC 后验「尖峰」:取**峰值帧的 canonical 后验**(同音素多写法取最大,如 ah/ax)作为最佳证据。
        gop_raw = max(float(seg[:, c].max()) for c in canon_ids)
        score01 = calibrate(gop_raw, cal)
        st = pron_status(score01, cal)
        # canonical 的**全局最佳**(整段任意帧):模型到底能不能在这段录音里听到这个音 + 在第几帧。
        best_c = max(canon_ids, key=lambda c: float(emission[:, c].max()))
        gmax_raw = float(emission[:, best_c].max())
        peak_t = round(int(emission[:, best_c].argmax()) * frame_sec, 3)

        reliable = True
        actual_ph = None
        hint = None
        if st == PRON_BAD:
            # 段内最强竞争音素(非 canon、非 blank)。
            peakc = seg.max(dim=0).values.clone()
            for c in canon_ids:
                peakc[c] = -1e30
            if 0 <= _blank_id < peakc.shape[0]:
                peakc[_blank_id] = -1e30
            other_id = int(peakc.argmax())
            other_raw = float(peakc[other_id])
            # **安全口径**:仅当「canonical 在段内基本没发出」**且**「段内也没有任何明确替代音」
            # 时,才判 uncertain(连"听到了什么"都说不上 → 多半没对齐上)。
            # 只要有明确替代音(如 /θ/ 段里有 /s/),一律保留 bad —— 绝不漏判真错读。
            if gop_raw < UNCERTAIN_CANON_FLOOR and other_raw < CONFIDENT_OTHER_RAW:
                st = PRON_UNCERTAIN
                reliable = False
                hint = "引擎没把这个音对齐好(可能没听清),不计作读错"
            else:
                actual_tok = model_token_to_arpabet(_vocab_inv.get(other_id, ""))
                if actual_tok and actual_tok != ph:
                    actual_ph = actual_tok
                hint = build_hint(ph, actual_ph)
        _ = gmax_raw  # 全局峰仅诊断参考,不再据其判 uncertain(会漏判真替换错读)
        phone_evals.append(
            PhoneEval(ph=ph, score=round(score01, 4), status=st,
                      expected_ph=ph if st in (PRON_BAD, PRON_WARN) else None,
                      actual_ph=actual_ph, hint=hint, reliable=reliable,
                      t_start=round(sp.start * frame_sec, 3),
                      t_end=round((sp.end + 1) * frame_sec, 3),
                      peak_t=peak_t, gop_raw=round(gop_raw, 3))
        )

    # ---- 聚合到词(剔除 uncertain:没对齐上的音素不参与词分,不冤枉用户)----
    words: list[WordEval] = []
    for wi, disp in enumerate(display_words):
        ph_evals = [pe for j, pe in enumerate(phone_evals)
                    if pe is not None and owner_word[j] == wi]
        if not ph_evals:
            # 该词无可评音素(G2P 未登录) → 发音维度留空。
            words.append(WordEval(ref=disp, score=0.0, status=PRON_WARN))
            continue
        reliable_evals = [pe for pe in ph_evals if pe.status != PRON_UNCERTAIN]
        if not reliable_evals:
            # 整词都没对齐上 → 词也标 uncertain(不计分、不拦通过),但 phones 仍透出供展示。
            words.append(WordEval(ref=disp, score=0.0, status=PRON_UNCERTAIN, phones=ph_evals))
            continue
        wscore = aggregate_word([pe.score for pe in reliable_evals])
        words.append(WordEval(ref=disp, score=wscore,
                              status=pron_status(wscore, cal), phones=ph_evals))

    resp = assemble_response(ref_text, words, None, MODEL_ID, granularity)
    _debug_dump(wav, ref_text, resp)
    return resp


def _debug_dump(wav, ref_text: str, resp: dict) -> None:
    """调试:env `GOP_DEBUG_DIR` 设了就把「模型实际听到的 16k wav + 逐音素结果」存盘。
    用于排查"分数过不去"是音频问题还是判分太严。生产不设此 env 即 no-op。"""
    dbg = os.environ.get("GOP_DEBUG_DIR")
    if not dbg:
        return
    try:
        import time
        import wave

        import numpy as np

        os.makedirs(dbg, exist_ok=True)
        ts = time.strftime("%Y%m%d-%H%M%S")
        base = os.path.join(dbg, f"assess-{ts}")
        arr = wav.numpy() if hasattr(wav, "numpy") else np.asarray(wav)
        pcm = (np.clip(arr, -1, 1) * 32767).astype(np.int16)
        with wave.open(base + ".wav", "wb") as wf:
            wf.setnchannels(1)
            wf.setsampwidth(2)
            wf.setframerate(TARGET_SR)
            wf.writeframes(pcm.tobytes())
        with open(base + ".json", "w", encoding="utf-8") as f:
            json.dump({"ref_text": ref_text, "result": resp}, f, ensure_ascii=False, indent=2)
    except Exception:  # noqa: BLE001
        pass


if __name__ == "__main__":
    # 调试入口:python gop.py <audio> <ref_text> [granularity]
    import sys

    _audio = sys.argv[1]
    _ref = sys.argv[2]
    _gran = sys.argv[3] if len(sys.argv) > 3 else "word"
    print(json.dumps(assess(_audio, _ref, {"granularity": _gran}), ensure_ascii=False, indent=2))
