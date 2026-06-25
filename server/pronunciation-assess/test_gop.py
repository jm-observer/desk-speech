"""gop.py 纯函数单测(不依赖 torch/transformers/g2p_en)。

覆盖标定曲线、状态分档、分词、聚合、hint 拼接、响应组装(含 granularity 裁剪)。
运行:python server/pronunciation-assess/test_gop.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import gop  # noqa: E402


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


def test_assemble_response_sentence_granularity_omits_phones():
    words = [
        gop.WordEval(ref="think", score=0.42, status=gop.PRON_BAD, phones=[
            gop.PhoneEval(ph="TH", score=0.18, status=gop.PRON_BAD),
        ]),
    ]
    resp = gop.assemble_response("think", words, transcript=None,
                                 model_id="m", granularity="sentence")
    # sentence 粒度省略 phones[],但 bad_phone_count 仍据音素算。
    assert "phones" not in resp["words"][0]
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


def test_calibration_load_missing_returns_default():
    cal = gop.Calibration.load(None)
    assert cal.a == 4.0 and cal.b == -1.0
    cal2 = gop.Calibration.load("/no/such/file.json")
    assert cal2.ok_min == 0.70


CASES = [
    test_calibrate_monotonic_and_bounds,
    test_pron_status_thresholds,
    test_tokenize_and_strip_stress,
    test_aggregate_penalizes_worst,
    test_build_hint,
    test_model_token_candidates_covers_ipa_and_arpabet,
    test_model_token_to_arpabet_roundtrip,
    test_assemble_response_word_granularity,
    test_assemble_response_sentence_granularity_omits_phones,
    test_calibration_load_missing_returns_default,
]


if __name__ == "__main__":
    for case in CASES:
        case()
        print(f"ok: {case.__name__}")
    print(f"\n{len(CASES)} passed")
