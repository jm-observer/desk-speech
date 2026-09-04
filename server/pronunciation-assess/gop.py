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

# 词尾辅音容忍:两类**标准语流现象**让词尾辅音的声学证据天然不可靠,被判 bad 一律降为
# uncertain(存疑不计错),绝不因此拦通过(见 toolkit docs/english-shadow-todo.md「标定实测发现
# 2026-07-02」,母语 TTS 的句尾 /t/、"is this" 的 /z/ 均被冤枉):
# ① 塞音不除阻(unreleased stop):词尾塞音常只闭塞不爆破,几乎没有爆破证据,模型分不清
#    「不除阻(标准)」与「吞音(错读)」。
# ② 浊阻音清化(final devoicing):词尾浊阻音普遍部分清化、跨词界与后续辅音同化
#    (is this → /ɪz̥ ðɪs/),清化落在其合法清对应音上仍是标准读法。
# 注意:**清擦音不豁免**(TH/S/F 等):path→pass(θ→s)是换发音位置的真错读,必须仍能判。
WORD_FINAL_STOPS = {"P", "B", "T", "D", "K", "G"}
WORD_FINAL_VOICED_OBSTRUENTS = {"B", "D", "G", "Z", "V", "DH", "ZH", "JH"}

# 峰偏移豁免:被判 bad 的音素,若其 canonical 在**整段录音的别处**有清晰峰(≥ PEAK_STRONG_RAW)
# 且峰离对齐段 > PEAK_OFFSET_TOL 秒 → 用户大概率发出了这个音,只是慢读/停顿把对齐窗挤错了位
# (实测用户慢读时 shoes 的 UW/Z 峰偏出 +0.5s 落进下一词的窗)。判 uncertain 而非错读。
# 风险:同一音素在句中他词也出现时可能被误豁免 → 只豁免为「存疑」(不给分),不豁免为 ok。
PEAK_STRONG_RAW = -1.5
PEAK_OFFSET_TOL = 0.25

# 「整段都没听到这个音」判据。canonical 在**整段录音任意帧**的最佳后验都低于此 → 用户根本
# 没发出这个音(吞音,或整体换成了别的音),这**不是对齐问题**。
#
# 为什么必须单独判(2026-08-31 实测 delicious):底下那条"段内没证据 + 段内也没明确替代音 →
# uncertain"的兜底,会把**吞音**也一并放过——告诉用户"引擎没对齐好,不算你错"。实测本人连读
# delicious 时 /l/(全局峰 -2.7~-3.7)与 /ʃ/(-5.9~-6.5)整段都不存在(模型逐帧听到的是
# d-iy-d-iy-s-ao-s),却因这条兜底被判存疑不计错——**放水方向的漏判**,比冤枉更难发现:
# 用户以为自己只是"没对齐上",一直照错的读法练。
#
# 阈值取得比 PEAK_STRONG_RAW 更保守(-3.0 vs -1.5):这一档要把 uncertain 升级成 bad,宁可
# 漏判几个,不可冤枉标准发音。母语 TTS 回归(regression_standard.py)是这条的守门人。
NOT_HEARD_PEAK_RAW = -3.0



def not_heard_anywhere(gmax_raw: float) -> bool:
    """canonical 在整段录音里最好的一帧也这么低 → 这个音根本没发出来(不是没对齐上)。"""
    return gmax_raw < NOT_HEARD_PEAK_RAW


# 辅音丛简化(cluster reduction):夹在两个辅音之间的塞音,母语者**普遍整个删掉**——
# products /prɑdəkts/ 读成 prah-duks、asked /æskt/ 读成 ast、next day 读成 neks day。
# 这是标准语流,不是吞音错误。
#
# 为什么必须单列(2026-08-31 A/B 实测):`not_heard_anywhere` 上线后,母语 TTS 的 products
# 里 /t/ 整段确实听不到(peak −5.0/−5.3),被新判据判成"吞掉了" → en_f_5895 的句分从 0.904
# 砸到 0.735。「整段没听到」这个事实是对的,但结论错了:它本来就不该被读出来。
# 与 `final_stop_leniency`(词尾塞音不除阻)是同一类豁免,只是位置不同。
CLUSTER_STOPS = {"P", "B", "T", "D", "K", "G"}


def word_neighbor_phones(flat_phones: list, owner_word: list, i: int) -> tuple:
    """取第 `i` 个音素在**同一个词内**的前后邻居(跨词或越界 → None)。

    单独抽出来是为了能被测到:辅音丛豁免的正确性一半在 `cluster_stop_leniency` 的判据、
    另一半在"邻居怎么取"——后者如果退回成直接取展平序列的前后项,跨词的音就会被当成丛内。
    """
    prev_ph = (flat_phones[i - 1]
               if i > 0 and owner_word[i - 1] == owner_word[i] else None)
    next_ph = (flat_phones[i + 1]
               if i + 1 < len(flat_phones) and owner_word[i + 1] == owner_word[i] else None)
    return prev_ph, next_ph


def cluster_stop_leniency(ph: str, prev_ph: Optional[str], next_ph: Optional[str]) -> bool:
    """塞音夹在两个辅音之间(辅音丛内)→ 删除是标准读法,不判吞音。

    只豁免**塞音**:辅音丛里的擦音(products 的 /s/、asked 的 /s/)不会被母语者删掉,
    真丢了就是真丢了。

    **邻居必须同属一个词**(调用方负责,跨词一侧传 None):`next place` 展平后是
    `…t s | p l…`,若拿展平序列的前后音素来判,词首的 /p/ 会被认成"夹在 s 与 l 之间"而
    豁免掉——它其实是词首塞音,吞了就是吞了。词尾塞音另有 `final_stop_leniency` 覆盖。
    """
    base = strip_stress(ph)
    if base not in CLUSTER_STOPS:
        return False
    if prev_ph is None or next_ph is None:
        return False
    return (strip_stress(prev_ph) not in ARPABET_VOWELS
            and strip_stress(next_ph) not in ARPABET_VOWELS)


def final_stop_leniency(ph: str, word_final: bool) -> bool:
    """词尾塞音/浊阻音被判 bad 时是否豁免为 uncertain(不除阻/清化是标准读法,模型分不清)。"""
    base = strip_stress(ph)
    return word_final and (base in WORD_FINAL_STOPS or base in WORD_FINAL_VOICED_OBSTRUENTS)


def peak_elsewhere(gmax_raw: float, peak_t: float, t_start: float, t_end: float) -> bool:
    """canonical 全局峰清晰、且明显偏出对齐段 → 疑似对齐错位(慢读/停顿),豁免为 uncertain。"""
    if gmax_raw < PEAK_STRONG_RAW:
        return False
    return peak_t < t_start - PEAK_OFFSET_TOL or peak_t > t_end + PEAK_OFFSET_TOL


# 弱读元音容忍:自然语流中,非重读元音与闭类虚词的元音普遍**弱化为 ə**(vowel reduction,
# 完全标准:What is it → /wət əz ət/),与词典 canonical 元音不一致会被误判 bad。实测母语 TTS
# 的 What/AH、it/IH、is/IH 均被冤枉(CMUdict 给虚词标 IH1/AH1,不能只看重音标记)。
# 重读的**实义词**元音不豁免:ship/sheep(IH/IY)这类真元音错读必须仍能判。
ARPABET_VOWELS = {"AA", "AE", "AH", "AO", "AW", "AY", "EH", "ER", "EY",
                  "IH", "IY", "OW", "OY", "UH", "UW"}
# 闭类虚词(冠词/介词/连词/助动词/代词/疑问词):语流中常整词弱读。
FUNCTION_WORDS = {
    "a", "an", "the",
    "of", "to", "in", "on", "at", "for", "from", "by", "with",
    "and", "or", "but", "as", "than", "if",
    "is", "am", "are", "was", "were", "be", "been", "being",
    "do", "does", "did", "can", "could", "will", "would", "shall", "should",
    "may", "might", "must", "have", "has", "had",
    "it", "its", "you", "your", "he", "she", "we", "they",
    "them", "him", "her", "us", "me", "my", "i", "our", "their",
    "what", "this", "that", "these", "those",
}


def reduced_vowel_leniency(ph_raw: str, word: str) -> bool:
    """元音被判 bad 时是否豁免为 uncertain:非重读(ARPAbet 重音 0)或所在词是闭类虚词。

    `ph_raw` 为**保留重音数字**的 ARPAbet(如 `IH0`);辅音一律不豁免(th→s 等必须仍能判)。
    """
    if strip_stress(ph_raw) not in ARPABET_VOWELS:
        return False
    return ph_raw.strip().endswith("0") or word.lower().strip("'") in FUNCTION_WORDS


@dataclass
class Calibration:
    """打分量 → 0~1 的标定 + 状态分档阈值。

    **输入是 `gop_eff`**(段内 canonical 峰 − 竞争惩罚,见 `assess`),不是响应里透出的
    `gop_raw`(那只是 canonical 峰,给诊断看的)。两者同为 (-inf,0]、0=完美,但不是一个量。

    标定曲线:`score01 = sigmoid(a * (gop_eff - b))`。`a` 控制陡度,`b` 是 0.5 分对应的
    打分量(中点)。**应由 speechocean762 拟合**(见 README「标定」),此处默认值与仓库
    `calibration.json` 都还是按真机后验**手调**的参考值,正式版须用真实数据重标。
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


def calibrate(gop_eff: float, cal: Calibration) -> float:
    """打分量(`gop_eff`,含竞争惩罚)→ 0~1。数值钳到 [0,1]。"""
    z = cal.a * (gop_eff - cal.b)
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


# 模型 vocab 里的**非标准 ARPAbet 写法** → 标准 ARPAbet。L2 模型的 93 个 token 里有
# `ax`(schwa,标准 ARPAbet 归入 AH)、`eu`、`o`、`ts` 这几个额外符号;不归一的话它们会以
# 原始小写形态混进 `heard` 透到前端(一串大写里夹个 `ax`,看着像 bug)。
# 特殊符号(`<pad>`/`<s>`/`</s>`/`<unk>`)映射为空 = 不进 heard。
_TOKEN_ALIAS = {
    "AX": "AH",   # schwa
    "EU": "UH",   # 圆唇央元音,最近的标准 ARPAbet
    "O": "OW",
    "TS": "T",    # 塞擦 /ts/,归到最近的标准音
}
_TOKEN_DROP = {"<PAD>", "<S>", "</S>", "<UNK>"}


def model_token_to_arpabet(tok: str) -> str:
    """模型 vocab token(IPA 或 ARPAbet)→ 标准 ARPAbet。特殊符号 → 空串。

    IPA 模型:查 IPA→ARPAbet 反表;ARPAbet 模型:去重音大写即可;模型特有写法查别名表;
    查不到回退原样。
    """
    if tok in _IPA_ARPABET:
        return _IPA_ARPABET[tok]
    su = strip_stress(tok)
    if su in _TOKEN_DROP:
        return ""
    if su in _ARPABET_IPA:  # 本就是 ARPAbet
        return su
    if su in _TOKEN_ALIAS:
        return _TOKEN_ALIAS[su]
    return tok


def strip_err_marker(tok: str) -> str:
    """剥掉 L2 模型的误读/变体标记:`ih_err`→`ih`、`b*`→`b`。用于把"误读变体"还原成基础音素。"""
    t = re.sub(r"_err$", "", tok, flags=re.IGNORECASE)
    return t.rstrip("*")


def build_hint(expected_ph: str, actual_ph: Optional[str]) -> str:
    """据「期望 vs 实际」音素拼人类可读纠音文案。`actual` 为空(漏读/无替代)时给通用提示。"""
    exp_ipa = arpabet_to_ipa(expected_ph)
    if actual_ph and strip_stress(actual_ph) != strip_stress(expected_ph):
        return f"/{exp_ipa}/ 读成了 /{arpabet_to_ipa(actual_ph)}/"
    return f"/{exp_ipa}/ 发音偏弱"


def greedy_tokens(argmax_ids: list, blank_id: int) -> list:
    """CTC greedy 解码:逐帧 argmax → 合并连续重复 + 去 blank,返回 `[(帧号, token_id)]`。

    这是「模型到底听到了什么」的原始形态。判分链路只问"期望音素在不在",答不了用户真正
    想知道的"那我读成什么了";把它单独透出来,错读才从"分低"变成"可照着改"。
    """
    out = []
    prev = None
    for t, a in enumerate(argmax_ids):
        a = int(a)
        if a != prev and a != blank_id:
            out.append((t, a))
        prev = a
    return out


def assign_heard_to_words(heard: list, word_ranges: list) -> list:
    """把 greedy 出来的 `[(帧, 音素)]` 按帧落到各词的时间范围里。

    `word_ranges[i]` = 该词首末音素对齐段的 **半开**区间 `[起帧, 止帧)`——torchaudio 的
    `TokenSpan.end` 是**排他上界**(实测:帧 1,2 属同一 token → `start=1, end=3`,其内部也按
    `scores[start:end]` 求均值)。按闭区间用 `end` 会把下一段的第一帧吃进来。
    落在所有词之外的帧丢弃(词间停顿/噪声)。对齐本身错位时 heard 也会跟着错配——它是
    **诊断展示**,不参与判分。
    """
    out = [[] for _ in word_ranges]
    for frame, ph in heard:
        for wi, rng in enumerate(word_ranges):
            if rng is None:
                continue
            lo, hi = rng
            if lo <= frame < hi:
                out[wi].append(ph)
                break
    return out


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
    # 诊断:对齐段内 canonical 的峰值 log 后验(≤0,越接近 0 证据越强)。**不是送进标定的
    # 打分量**——那是含竞争惩罚的 gop_eff(见 assess 与 README「标定」)。
    gop_raw: Optional[float] = None
    # 诊断:canonical 在**整段录音**的最佳后验(peak_t 处的值)。**必须与 peak_t 一起看**——
    # 只看 peak_t 的位移会得出"对齐错位"的错误结论:峰值弱时那只是"整段里最不差的一帧",
    # 不代表用户在那儿发出了这个音(2026-08-31 实测踩过)。
    peak_raw: Optional[float] = None


@dataclass
class WordEval:
    ref: str
    score: float
    status: str
    phones: list[PhoneEval] = field(default_factory=list)
    # 模型在这个词的时间范围里**实际听到**的音素串(ARPAbet)。诊断用,不参与判分。
    heard: list[str] = field(default_factory=list)


# 强制对齐产出校验。CTC forced_align 的 spans 必须与展平后的目标音素**逐个同 id、等长**——
# 这是后续 `spans[i] ↔ flat_phones[i]` 配对打分的前提。一旦不成立,错位是**静默**的:
# 每个音素拿到别人的时间段,分数全烂但无任何报错。
#
# 实际踩过的坑(2026-08-20):`merge_tokens` 的 blank 参数默认 0,而本模型 vocab 里 id 0 是音素
# `aa`、真 blank(<pad>)是 89。不显式传 blank → 所有 /ɑ/ 的 span 被当 blank 剔除、真 blank 段
# 反被保留,从句中第一个 /ɑ/ 起全体错位,句尾音素直接没有 span(milk 的 L/K 丢失)。表现为
# 「单词级评测正常、整句评测崩盘」,连母语 TTS 的整句也只有 0.13 分。
def spans_match_targets(span_tokens: list[int], target_ids: list[int]) -> bool:
    """spans 的 token 序列是否与目标音素 id 序列逐个一致(等长且同序)。"""
    return len(span_tokens) == len(target_ids) and all(
        a == b for a, b in zip(span_tokens, target_ids)
    )


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
        if w.heard:
            wd["heard"] = w.heard
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
    if p.peak_raw is not None:
        d["peak_raw"] = p.peak_raw
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
    """单词 → ARPAbet 音素序列(**保留重音数字**,如 `IH0`,供弱读元音容忍判据;
    展示/对齐处再 strip)。OOV 由 g2p_en 的 seq2seq 兜底。"""
    phones = [p for p in g2p(word) if p.strip() and re.match(r"[A-Za-z]", p)]
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
    flat_phones: list[str] = []   # ARPAbet 形(去重音),供展示 / hint
    flat_raw: list[str] = []      # ARPAbet 原形(带重音数字),供弱读元音容忍判据
    flat_ids: list[int] = []      # 对齐目标:模型 vocab 主 id
    flat_idsets: list[list] = []  # 打分用:同音素可接受的全部 id(取后验最大)
    owner_word: list[int] = []
    for wi, phs in enumerate(word_phones):
        for ph in phs:
            ids = _resolve_token_ids(ph)
            if ids:
                flat_phones.append(strip_stress(ph))
                flat_raw.append(ph)
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
    # **必须显式传 blank**:merge_tokens 的 blank 默认值是 0,而本模型的 blank(<pad>)是 89,
    # id 0 是音素 `aa`——不传就会把所有 /ɑ/ 的 span 当 blank 剔掉,而把真 blank 段全部留下,
    # 导致 spans 与 flat_phones 静默错位(详见 spans_match_targets 上方的注释)。
    spans = torchaudio.functional.merge_tokens(aligned[0], scores[0], blank=_blank_id)
    # 合并后的 spans 对应非 blank 目标,顺序与 flat_phones 一致。
    spans = [s for s in spans if s.token != _blank_id]
    # 自检:错位是**静默**的(不报错、只是分数烂),换模型/换 torchaudio 版本都可能再犯。
    if not spans_match_targets([int(s.token) for s in spans], flat_ids):
        raise RuntimeError(
            "forced_align 产出的 spans 与目标音素序列不一致"
            f"(spans={len(spans)} targets={len(flat_ids)} blank={_blank_id});"
            "多半是 blank id 与某个音素 token id 冲突,或 torchaudio 对齐 API 行为变更"
        )

    # 帧→秒(供时间段);emission T 帧覆盖整段 wav。
    frame_sec = (wav.shape[0] / emission.shape[0]) / TARGET_SR

    # ---- 逐音素 GOP + 对齐可靠性 ----
    def _twin_claims_peak(idx: int, pt: float) -> bool:
        """峰偏移豁免的护栏:全局峰若落在**同一音素的其他对齐段**内(±50ms),说明那是句中
        另一处该音(如 "It is a ship" 里 It/is 的 IH),不是本音素错位——不得豁免。
        否则 sheep 读成 ship 会借别的词的 IH 峰混过(实测漏判案例)。"""
        for j, sp2 in enumerate(spans):
            if j == idx or j >= len(flat_phones) or flat_phones[j] != flat_phones[idx]:
                continue
            if sp2.start * frame_sec - 0.05 <= pt <= sp2.end * frame_sec + 0.05:
                return True
        return False

    phone_evals: list[Optional[PhoneEval]] = []
    for i, ph in enumerate(flat_phones):
        if i >= len(spans):
            phone_evals.append(None)
            continue
        sp = spans[i]
        # `TokenSpan.end` 是**排他上界**——torchaudio 内部也按 `scores[start:end]` 求均值
        # (实测 2.11:帧 1,2 属同一 token → start=1,end=3)。原先写成 `sp.end + 1` 多吃一帧,
        # 面板上每个音素都显示 40ms 宽、`It` 的 IH/T 区间还会假重叠,就是这么来的。
        #
        # **实测这批样本上分数没变**(2026-08-31 固定音频 A/B:8 短语×2 母语音色 + 5 错读探针 +
        # 3 条真人录音,24 条逐条同分)。这只是样本结论,不是算法保证——相邻 token 之间未必有
        # blank,多出的那帧仍可能抬高 canonical 峰或某个竞争音。所以取语义正确的半开区间,
        # 别再"为了保险"加回 +1;真要改窗口,重跑 A/B 看母语与错读探针两头。
        seg = emission[sp.start:sp.end]  # (n, C)
        if seg.shape[0] == 0:
            phone_evals.append(None)
            continue
        canon_ids = flat_idsets[i]
        # CTC 后验「尖峰」:取**峰值帧的 canonical 后验**(同音素多写法取最大,如 ah/ax)作为最佳证据。
        gop_raw = max(float(seg[:, c].max()) for c in canon_ids)
        # 竞争惩罚(标准 GOP 的后验比思想):段内最强**竞争 token**(排除 blank、canonical 自身、
        # 相邻两个目标音素——协同发音溢出不算竞争)若强过 canonical,按差距扣分。canonical 自己
        # 最强时零惩罚(正确发音打分与纯 canonical 后验完全一致)。只看 canonical 绝对后验会漏判
        # ship/sheep 这类近邻音替换:模型对 [i] 帧的 IH 后验也不低,但 IY 更高——差距即证据。
        comp = seg.max(dim=0).values.clone()
        excl = set(canon_ids)
        if i > 0:
            excl.update(flat_idsets[i - 1])
        if i + 1 < len(flat_idsets):
            excl.update(flat_idsets[i + 1])
        for c in excl:
            if 0 <= c < comp.shape[0]:
                comp[c] = -1e30
        if 0 <= _blank_id < comp.shape[0]:
            comp[_blank_id] = -1e30
        competitor_raw = float(comp.max())
        gop_eff = gop_raw - max(0.0, competitor_raw - gop_raw)
        score01 = calibrate(gop_eff, cal)
        st = pron_status(score01, cal)
        # canonical 的**全局最佳**(整段任意帧):模型到底能不能在这段录音里听到这个音 + 在第几帧。
        best_c = max(canon_ids, key=lambda c: float(emission[:, c].max()))
        gmax_raw = float(emission[:, best_c].max())
        peak_t = round(int(emission[:, best_c].argmax()) * frame_sec, 3)
        ts = round(sp.start * frame_sec, 3)
        te = round(sp.end * frame_sec, 3)
        # 词尾 = 本音素是所属词的最后一个音素(含句尾)。
        word_final = i + 1 >= len(owner_word) or owner_word[i + 1] != owner_word[i]

        reliable = True
        actual_ph = None
        hint = None
        if st == PRON_BAD and final_stop_leniency(ph, word_final):
            # 词尾塞音不除阻是标准读法,模型分不清「不除阻」与「吞音」→ 不判错(见纯函数区注释)。
            st = PRON_UNCERTAIN
            reliable = False
            hint = f"词尾 /{arpabet_to_ipa(ph)}/ 在语流中常不除阻/清化(标准读法),不计作读错"
        elif st == PRON_BAD and reduced_vowel_leniency(flat_raw[i], display_words[owner_word[i]]):
            # 非重读/虚词元音在语流中标准地弱化为 ə,与词典元音不符不算错(见纯函数区注释)。
            st = PRON_UNCERTAIN
            reliable = False
            hint = f"非重读 /{arpabet_to_ipa(ph)}/ 常弱化为 ə(标准语流),不计作读错"
        elif (st == PRON_BAD and peak_elsewhere(gmax_raw, peak_t, ts, te)
              and not _twin_claims_peak(i, peak_t)):
            # 该音在录音别处有清晰峰(且不是句中同音素的另一处)→ 多半是慢读/停顿把对齐窗
            # 挤错位,不冤枉用户。
            st = PRON_UNCERTAIN
            reliable = False
            hint = f"该音在 {peak_t}s 处有清晰峰,对齐可能错位,不计作读错"
        elif st == PRON_BAD:
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
                # 邻居只认**同一个词内**的:跨词的相邻音素不构成辅音丛(见函数注释)。
                prev_ph, next_ph = word_neighbor_phones(flat_phones, owner_word, i)
                if not_heard_anywhere(gmax_raw) and not cluster_stop_leniency(ph, prev_ph, next_ph):
                    # 段内没有、**整段任何一帧也没有** → 不是没对齐上,是这个音压根没发出来。
                    # 保留 bad,并把话说清楚:再说"引擎没对齐好"就是放水(见常量处注释)。
                    hint = (f"整段录音里都没听到 /{arpabet_to_ipa(ph)}/"
                            "(可能吞掉了,或整个音换成了别的音)")
                elif not_heard_anywhere(gmax_raw):
                    # 辅音丛内的塞音被删掉是标准读法(products → prah-duks),不算错。
                    st = PRON_UNCERTAIN
                    reliable = False
                    hint = (f"辅音丛里的 /{arpabet_to_ipa(ph)}/ 在语流中常整个省略"
                            "(标准读法),不计作读错")
                else:
                    st = PRON_UNCERTAIN
                    reliable = False
                    hint = "引擎没把这个音对齐好(可能没听清),不计作读错"
            else:
                # 段内最强竞争音 → 还原成基础音素(剥 L2 模型的 _err / * 标记)。
                other_base = model_token_to_arpabet(strip_err_marker(_vocab_inv.get(other_id, "")))
                if not other_base or strip_stress(other_base) == strip_stress(ph):
                    # 竞争音是 canonical 自己的「误读变体」(如 IH 的 ih_err)→ **发音不准**,非替换。
                    actual_ph = None
                    hint = f"/{arpabet_to_ipa(ph)}/ 发音不够准"
                else:
                    # 竞争音是**另一个**音素 → 真替换错读(如 /θ/→/s/)。
                    actual_ph = other_base
                    hint = build_hint(ph, actual_ph)
        phone_evals.append(
            PhoneEval(ph=ph, score=round(score01, 4), status=st,
                      expected_ph=ph if st in (PRON_BAD, PRON_WARN) else None,
                      actual_ph=actual_ph, hint=hint, reliable=reliable,
                      t_start=ts, t_end=te,
                      peak_t=peak_t, gop_raw=round(gop_raw, 3),
                      peak_raw=round(gmax_raw, 3))
        )

    # ---- 模型实际听到的音素串(CTC greedy),按词切分 ----
    # 判分只回答"期望的音在不在",答不了"那我读成什么了"。把 greedy 结果按词透出来,用户
    # 才看得到 delicious 被读成了 d-iy-d-iy-s-ao-s 这种事(2026-08-31 实测形态)。
    heard_raw = greedy_tokens(emission.argmax(dim=-1).tolist(), _blank_id)
    heard_phones = [
        (t, model_token_to_arpabet(strip_err_marker(_vocab_inv.get(tid, ""))))
        for t, tid in heard_raw
    ]
    heard_phones = [(t, ph) for t, ph in heard_phones if ph]
    word_ranges: list = []
    for wi in range(len(display_words)):
        idxs = [j for j in range(len(spans)) if j < len(owner_word) and owner_word[j] == wi]
        if idxs:
            word_ranges.append((spans[idxs[0]].start, spans[idxs[-1]].end))
        else:
            word_ranges.append(None)
    heard_by_word = assign_heard_to_words(heard_phones, word_ranges)

    # ---- 聚合到词(剔除 uncertain:没对齐上的音素不参与词分,不冤枉用户)----
    words: list[WordEval] = []
    for wi, disp in enumerate(display_words):
        ph_evals = [pe for j, pe in enumerate(phone_evals)
                    if pe is not None and owner_word[j] == wi]
        if not ph_evals:
            # 该词无可评音素(G2P 未登录) → 发音维度留空。
            words.append(WordEval(ref=disp, score=0.0, status=PRON_WARN,
                                  heard=heard_by_word[wi]))
            continue
        reliable_evals = [pe for pe in ph_evals if pe.status != PRON_UNCERTAIN]
        if not reliable_evals:
            # 整词都没对齐上 → 词也标 uncertain(不计分、不拦通过),但 phones 仍透出供展示。
            words.append(WordEval(ref=disp, score=0.0, status=PRON_UNCERTAIN,
                                  phones=ph_evals, heard=heard_by_word[wi]))
            continue
        wscore = aggregate_word([pe.score for pe in reliable_evals])
        words.append(WordEval(ref=disp, score=wscore, status=pron_status(wscore, cal),
                              phones=ph_evals, heard=heard_by_word[wi]))

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
