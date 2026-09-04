"""gop.py 纯函数单测(不依赖 torch/transformers/g2p_en)。

覆盖标定曲线、状态分档、分词、聚合、hint 拼接、响应组装(含 granularity 裁剪)。
运行:python server/pronunciation-assess/test_gop.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import gop  # noqa: E402


def test_spans_match_targets():
    # 正常:逐个同 id、等长。
    assert gop.spans_match_targets([22, 41, 85], [22, 41, 85])
    # 缺项(blank 参数传错时 id 0 的 aa 被当 blank 剔掉的真实形态)→ 必须判不一致。
    assert not gop.spans_match_targets([22, 41], [22, 0, 41])
    # 等长但错位(整体前移)→ 也必须判不一致,这正是静默错位的形态。
    assert not gop.spans_match_targets([22, 41, 85], [22, 0, 41])
    # 多出项。
    assert not gop.spans_match_targets([22, 41, 85], [22, 41])
    # 空对空成立(整句 G2P 全未登录的退化路径,由调用方另行短路)。
    assert gop.spans_match_targets([], [])


def test_calibrate_monotonic_and_bounds():
    cal = gop.Calibration()
    # gop_raw=0(完美)→ 接近 1;越负越低;恒在 [0,1]。
    perfect = gop.calibrate(0.0, cal)
    mid = gop.calibrate(cal.b, cal)
    bad = gop.calibrate(-5.0, cal)
    assert 0.0 <= bad < mid < perfect <= 1.0
    assert abs(mid - 0.5) < 1e-6, mid  # 中点 b 对应 0.5
    assert gop.calibrate(-1e9, cal) == 0.0
    assert gop.calibrate(1e9, cal) > 0.99


def test_pron_status_thresholds():
    cal = gop.Calibration(ok_min=0.7, warn_min=0.45)
    assert gop.pron_status(0.95, cal) == gop.PRON_OK
    assert gop.pron_status(0.70, cal) == gop.PRON_OK
    assert gop.pron_status(0.55, cal) == gop.PRON_WARN
    assert gop.pron_status(0.45, cal) == gop.PRON_WARN
    assert gop.pron_status(0.20, cal) == gop.PRON_BAD


def test_tokenize_and_strip_stress():
    assert gop.tokenize_words("I think, so!") == ["I", "think", "so"]
    assert gop.tokenize_words("don't stop") == ["don't", "stop"]
    assert gop.strip_stress("IH1") == "IH"
    assert gop.strip_stress("ay0") == "AY"
    assert gop.strip_stress("TH") == "TH"


def test_aggregate_penalizes_worst():
    # 一个 0.1 的烂音素应把词分明显拉低(min 权重 0.4)。
    w = gop.aggregate_word([0.9, 0.9, 0.1])
    assert w < 0.7, w
    # 全好则接近 1。
    assert gop.aggregate_word([0.95, 0.95]) > 0.9
    assert gop.aggregate_word([]) == 0.0
    # 句级聚合同理。
    s = gop.aggregate_sentence([0.95, 0.2])
    assert s < gop.aggregate_sentence([0.95, 0.9])


def test_build_hint():
    assert gop.build_hint("TH", "S") == "/θ/ 读成了 /s/"
    # 实际音素与期望相同 / 缺失 → 通用「偏弱」文案。
    assert "偏弱" in gop.build_hint("NG", None)
    assert "偏弱" in gop.build_hint("IH", "IH")


def test_assemble_response_word_granularity():
    words = [
        gop.WordEval(ref="I", score=0.95, status=gop.PRON_OK,
                     phones=[gop.PhoneEval(ph="AY", score=0.95, status=gop.PRON_OK)]),
        gop.WordEval(ref="think", score=0.42, status=gop.PRON_BAD, phones=[
            gop.PhoneEval(ph="TH", score=0.18, status=gop.PRON_BAD,
                          expected_ph="TH", actual_ph="S", hint="/θ/ 读成了 /s/"),
            gop.PhoneEval(ph="IH", score=0.71, status=gop.PRON_OK),
        ]),
    ]
    resp = gop.assemble_response("I think", words, transcript="i think",
                                 model_id="m", granularity="word")
    assert resp["model"] == "m"
    assert resp["bad_phone_count"] == 1
    assert resp["transcript"] == "i think"
    assert resp["words"][1]["pron_status"] == "bad"
    # word 粒度保留 phones,且结构化字段透出。
    th = resp["words"][1]["phones"][0]
    assert th["expected_ph"] == "TH" and th["actual_ph"] == "S"
    assert th["hint"] == "/θ/ 读成了 /s/"
    # ok 音素不带 expected/actual/hint。
    assert "expected_ph" not in resp["words"][1]["phones"][1]


def test_assemble_response_always_includes_phones():
    words = [
        gop.WordEval(ref="think", score=0.42, status=gop.PRON_BAD, phones=[
            gop.PhoneEval(ph="TH", score=0.18, status=gop.PRON_BAD),
        ]),
    ]
    # 始终返回 phones[](明细表整句也需要),granularity 不再裁剪。
    resp = gop.assemble_response("think", words, transcript=None,
                                 model_id="m", granularity="sentence")
    assert resp["words"][0]["phones"][0]["ph"] == "TH"
    assert resp["bad_phone_count"] == 1
    # transcript 为 None 时不出现该键。
    assert "transcript" not in resp


def test_model_token_candidates_covers_ipa_and_arpabet():
    # TH:ARPAbet 原形在前,IPA θ 次之(命中 vitouphy IPA 模型)。
    assert gop.model_token_candidates("TH1") == ["TH", "θ"]
    # AH:高频音素,IPA 模型常并入 schwa → 需含 ə 候选。
    assert "ə" in gop.model_token_candidates("AH")
    # G:IPA ɡ(U+0261) 与 ASCII g 都要给(不同模型写法不同)。
    cands = gop.model_token_candidates("G")
    assert "ɡ" in cands and "g" in cands
    # ER:ɝ/ɚ 备选齐。
    assert "ɝ" in gop.model_token_candidates("ER")


def test_model_token_to_arpabet_roundtrip():
    # IPA → ARPAbet 反查(供 actual_ph)。
    assert gop.model_token_to_arpabet("θ") == "TH"
    assert gop.model_token_to_arpabet("s") == "S"
    assert gop.model_token_to_arpabet("ə") == "AH"
    # 本就是 ARPAbet → 原样(去重音大写)。
    assert gop.model_token_to_arpabet("TH") == "TH"
    # 未知 → 回退原样(不崩)。
    assert gop.model_token_to_arpabet("???") == "???"


def test_strip_err_marker():
    # L2 模型的误读/变体标记还原成基础音素。
    assert gop.strip_err_marker("ih_err") == "ih"
    assert gop.strip_err_marker("IH_ERR") == "IH"
    assert gop.strip_err_marker("b*") == "b"
    assert gop.strip_err_marker("s") == "s"


def test_calibration_load_missing_returns_default():
    cal = gop.Calibration.load(None)
    assert cal.a == 4.0 and cal.b == -1.0


def test_final_stop_leniency():
    # 词尾塞音(不除阻)→ 豁免;非词尾 → 不豁免。
    assert gop.final_stop_leniency("T", word_final=True)
    assert gop.final_stop_leniency("K", word_final=True)
    assert gop.final_stop_leniency("D", word_final=True)
    assert not gop.final_stop_leniency("T", word_final=False)   # 词中塞音照常判
    # 词尾浊阻音(清化/同化,如 is this 的 Z)→ 豁免。
    assert gop.final_stop_leniency("Z", word_final=True)
    assert gop.final_stop_leniency("V", word_final=True)
    assert gop.final_stop_leniency("DH", word_final=True)
    assert not gop.final_stop_leniency("Z", word_final=False)
    # 清擦音/元音不豁免(path→pass 的 θ→s 必须仍能判错)。
    assert not gop.final_stop_leniency("TH", word_final=True)
    assert not gop.final_stop_leniency("S", word_final=True)
    assert not gop.final_stop_leniency("F", word_final=True)
    assert not gop.final_stop_leniency("ER", word_final=True)


def test_reduced_vowel_leniency():
    # 非重读元音(重音 0)→ 豁免,不看词性。
    assert gop.reduced_vowel_leniency("IH0", "music")
    assert gop.reduced_vowel_leniency("AH0", "about")
    # 虚词元音:词典标重读(it→IH1 / what→AH1)也豁免(语流中整词弱读)。
    assert gop.reduced_vowel_leniency("IH1", "it")
    assert gop.reduced_vowel_leniency("IH1", "is")
    assert gop.reduced_vowel_leniency("AH1", "What")   # 大小写不敏感
    # 实义词的重读元音不豁免:ship/sheep 必须仍能判。
    assert not gop.reduced_vowel_leniency("IH1", "ship")
    assert not gop.reduced_vowel_leniency("IY1", "sheep")
    assert not gop.reduced_vowel_leniency("ER1", "hurt")
    # 辅音一律不豁免(即使在虚词里:th→s 必须仍能判)。
    assert not gop.reduced_vowel_leniency("DH", "the")
    assert not gop.reduced_vowel_leniency("TH1", "think")


def test_peak_elsewhere():
    # 清晰峰(≥ PEAK_STRONG_RAW)明显偏出对齐段 → 疑似对齐错位。
    assert gop.peak_elsewhere(-0.5, peak_t=1.91, t_start=1.37, t_end=1.41)   # 实测 shoes/UW 形态
    assert gop.peak_elsewhere(-0.5, peak_t=0.50, t_start=1.37, t_end=1.41)   # 偏前同理
    # 峰在段内 / 只偏一点(≤ 容忍)→ 不豁免。
    assert not gop.peak_elsewhere(-0.5, peak_t=1.39, t_start=1.37, t_end=1.41)
    assert not gop.peak_elsewhere(-0.5, peak_t=1.60, t_start=1.37, t_end=1.41)
    # 峰不清晰(如「I sink so」里根本没有 TH)→ 不豁免,真错读仍判 bad。
    assert not gop.peak_elsewhere(-4.0, peak_t=1.91, t_start=1.37, t_end=1.41)
    cal2 = gop.Calibration.load("/no/such/file.json")
    assert cal2.ok_min == 0.70


def test_model_token_alias_normalizes_nonstandard():
    """模型 vocab 有 4 个非标准写法(ax/eu/o/ts)+ 4 个特殊符号,不归一就会混进 heard 透给前端。"""
    assert gop.model_token_to_arpabet("ax") == "AH"      # schwa
    assert gop.model_token_to_arpabet("ax_err".replace("_err", "")) == "AH"
    assert gop.model_token_to_arpabet("o") == "OW"
    assert gop.model_token_to_arpabet("ts") == "T"
    # 特殊符号不进 heard。
    for sp in ("<pad>", "<s>", "</s>", "<unk>"):
        assert gop.model_token_to_arpabet(sp) == "", sp
    # 标准 ARPAbet / IPA 仍照旧。
    assert gop.model_token_to_arpabet("sh") == "SH"
    assert gop.model_token_to_arpabet("ʃ") == "SH"


def test_not_heard_anywhere():
    """整段都没听到 = 吞音,不该按"没对齐好"放过(2026-08-31 实测 delicious)。"""
    # 实测值:本人连读 delicious 时 /ʃ/ 的全局峰 -5.9~-6.5、/l/ -2.7~-3.7 —— 整段都没有这个音。
    assert gop.not_heard_anywhere(-6.2)
    assert gop.not_heard_anywhere(-3.7)
    # 正常发出的音(实测 -0.0~-0.6)绝不能命中,否则标准发音会被判吞音。
    assert not gop.not_heard_anywhere(-0.0)
    assert not gop.not_heard_anywhere(-0.6)
    assert not gop.not_heard_anywhere(-1.5)
    # 这一档比"清晰峰"更保守:处在两者之间的按"没对齐好"处理,宁可漏判不可冤枉。
    assert gop.NOT_HEARD_PEAK_RAW < gop.PEAK_STRONG_RAW


def test_cluster_stop_leniency():
    """辅音丛里的塞音被删是标准读法——母语 TTS 的 products 就整段没有 /t/(A/B 实测)。"""
    # products /p r AA d AH k t s/:T 夹在 K 与 S 之间 → 豁免。
    assert gop.cluster_stop_leniency("T", "K", "S")
    assert gop.cluster_stop_leniency("D", "N", "Z")
    # 一侧是元音 → 不是丛内,真丢了就是丢了(delicious 的 D-IH 不豁免)。
    assert not gop.cluster_stop_leniency("T", "K", "AH")
    assert not gop.cluster_stop_leniency("T", "AH", "S")
    # 词首/词尾(另一侧为 None)不走这条(词尾归 final_stop_leniency)。
    # **跨词的邻居由调用方传 None**:`next place` 的词首 /p/ 前面虽然是 /s/,也不算丛内,
    # 否则吞掉词首塞音会被当成标准 cluster reduction 放过。
    assert not gop.cluster_stop_leniency("T", None, "S")
    assert not gop.cluster_stop_leniency("P", None, "L")
    assert not gop.cluster_stop_leniency("T", "K", None)
    # 只豁免塞音:丛里的擦音/流音丢了就是真丢了(products 的 /s/、delicious 的 /l/)。
    assert not gop.cluster_stop_leniency("S", "K", "T")
    assert not gop.cluster_stop_leniency("L", "D", "R")


def test_word_neighbor_phones_stops_at_word_boundary():
    """辅音丛豁免的正确性一半在判据、一半在"邻居怎么取"——这里覆盖后者。

    `next place` 展平后是 `t s | p l`:词首的 /p/ 前面虽然紧挨着 /s/,那是**上一个词的**,
    不构成辅音丛。取邻居时不看 owner_word 的话,吞掉词首塞音会被当成标准 cluster reduction。
    """
    flat = ["T", "S", "P", "L", "EY", "S"]
    owner = [0, 0, 1, 1, 1, 1]          # next | place
    # 词首 /p/(下标 2):前邻跨词 → None,后邻同词 → L。
    assert gop.word_neighbor_phones(flat, owner, 2) == (None, "L")
    # 词尾 /s/(下标 1):后邻跨词 → None。
    assert gop.word_neighbor_phones(flat, owner, 1) == ("T", None)
    # 词内中间音素:两侧都在。
    assert gop.word_neighbor_phones(flat, owner, 3) == ("P", "EY")
    # 首尾越界。
    assert gop.word_neighbor_phones(flat, owner, 0) == (None, "S")
    assert gop.word_neighbor_phones(flat, owner, 5) == ("EY", None)
    # 串起来:这个 /p/ 不该被辅音丛豁免掉。
    prev_ph, next_ph = gop.word_neighbor_phones(flat, owner, 2)
    assert not gop.cluster_stop_leniency("P", prev_ph, next_ph)


def test_greedy_tokens():
    # 合并连续重复 + 去 blank,只保留每段的**起始帧**。
    assert gop.greedy_tokens([9, 9, 3, 3, 3, 9, 5, 9], blank_id=9) == [(2, 3), (6, 5)]
    # 同一 token 被 blank 打断 → 算两次(真的读了两遍)。
    assert gop.greedy_tokens([3, 9, 3], blank_id=9) == [(0, 3), (2, 3)]
    # 相邻不同 token 不合并。
    assert gop.greedy_tokens([3, 4, 4, 5], blank_id=9) == [(0, 3), (1, 4), (3, 5)]
    # 全 blank / 空 → 空。
    assert gop.greedy_tokens([9, 9, 9], blank_id=9) == []
    assert gop.greedy_tokens([], blank_id=9) == []


def test_assign_heard_to_words_uses_half_open_spans():
    """词范围是**半开**区间 [start, end) —— torchaudio 的 TokenSpan.end 是排他上界。

    实测(torchaudio 2.11):帧 1,2 属于同一 token 时,merge_tokens 给的是 start=1, end=3,
    其内部也按 `scores[start:end]` 取平均。按闭区间用 `end` 会把**下一段的第一帧**吃进来。
    """
    # 词1 = 帧[0,3) 即 0,1,2;词2 = 帧[3,5) 即 3,4。帧 3 必须归词2,不能被词1 吃掉。
    heard = [(0, "D"), (2, "IY"), (3, "S"), (4, "T")]
    assert gop.assign_heard_to_words(heard, [(0, 3), (3, 5)]) == [["D", "IY"], ["S", "T"]]
    # 落在末尾之后的帧不属于任何词。
    assert gop.assign_heard_to_words([(5, "N")], [(0, 3), (3, 5)]) == [[], []]


def test_assign_heard_to_words():
    # 两个词,各自的帧范围;落在范围外的(词间停顿/噪声)丢弃。
    heard = [(1, "D"), (5, "IY"), (9, "S"), (30, "N")]
    ranges = [(0, 6), (8, 12)]
    assert gop.assign_heard_to_words(heard, ranges) == [["D", "IY"], ["S"]]
    # 无可评音素的词(range=None)不参与分配,也不报错。
    assert gop.assign_heard_to_words(heard, [None, (8, 12)]) == [[], ["S"]]
    # 范围重叠时归第一个命中的词(不重复计入)。
    assert gop.assign_heard_to_words([(5, "IY")], [(0, 6), (4, 9)]) == [["IY"], []]


def test_assemble_response_includes_heard():
    """「读成了什么」必须能透到前端——这是错读从"分低"变成"可照着改"的唯一信息。"""
    w = gop.WordEval(ref="delicious", score=0.22, status=gop.PRON_BAD,
                     phones=[gop.PhoneEval(ph="L", score=0.0, status=gop.PRON_BAD)],
                     heard=["D", "IY", "D", "IY", "S", "AO", "S"])
    resp = gop.assemble_response("It is delicious", [w], None, "m", "sentence")
    assert resp["words"][0]["heard"] == ["D", "IY", "D", "IY", "S", "AO", "S"]
    # 没听到任何东西时不带这个字段(省带宽,前端据 in 判断)。
    w2 = gop.WordEval(ref="it", score=0.9, status=gop.PRON_OK)
    assert "heard" not in gop.assemble_response("it", [w2], None, "m", "sentence")["words"][0]


def test_phone_json_carries_peak_raw():
    """peak_t 必须与 peak_raw 一起给:只看峰的**位移**会把"整段都没这个音"误读成"对齐错位"
    ——2026-08-31 就是这么误判的(delicious 的 /l/ 峰在 2.03s,但那儿的后验只有 -2.8)。"""
    p = gop.PhoneEval(ph="L", score=0.0, status=gop.PRON_BAD,
                      t_start=1.449, t_end=1.49, peak_t=2.033, gop_raw=-4.644, peak_raw=-2.8)
    d = gop._phone_to_json(p)
    assert d["peak_t"] == 2.033 and d["peak_raw"] == -2.8


# 自动收集本模块所有 test_*：手工维护列表漏过 4 个测试(strip_err_marker / final_stop_leniency /
# reduced_vowel_leniency / peak_elsewhere 写了但从没跑过),这种"写了不跑"比没写更糟。
CASES = [
    test_spans_match_targets,
    test_calibrate_monotonic_and_bounds,
    test_pron_status_thresholds,
    test_tokenize_and_strip_stress,
    test_aggregate_penalizes_worst,
    test_build_hint,
    test_model_token_candidates_covers_ipa_and_arpabet,
    test_model_token_to_arpabet_roundtrip,
    test_assemble_response_word_granularity,
    test_assemble_response_always_includes_phones,
    test_calibration_load_missing_returns_default,
]
CASES = [fn for name, fn in sorted(globals().items())
         if name.startswith("test_") and callable(fn)]


if __name__ == "__main__":
    for case in CASES:
        case()
        print(f"ok: {case.__name__}")
    print(f"\n{len(CASES)} passed")
