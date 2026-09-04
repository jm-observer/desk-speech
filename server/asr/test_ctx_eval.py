"""前缀上下文解码离线评测的测试。

判据全部来自 docs/asr-ctx-prefix-postmortem.md 的 P0-b，逐条对应一个曾经踩过或
被指出的坑：

- 分母只取 would_apply：not_located 根本没有候选文本，rejected / unchanged 不改输出，
  把它们塞进分母只会稀释风险；
- 样本完整性要实际读 wav，不能只看文件在不在；
- 重复多轮时非改写轮次既不计好也不计坏，且至少要有一轮改写才算有效样本；
- 有效样本数本身也是通过线的一部分——只看「0 坏、≥5 好」，样本缩水到十几条照样通过。
"""

import json

import numpy as np

from ctx_eval import Verdict, aggregate, is_candidate, load_records, reclassify
from ctx_observe import CtxObserver

SR = 16000


def _write(root, **over):
    rec = {
        "key": over.pop("key", "sess-00001"),
        "stage": over.pop("stage", "would_apply"),
        "plain": over.pop("plain", "伪略一下这个文档。"),
        "prev_text": over.pop("prev_text", "你看一下识别记录。"),
        "full": over.pop("full", "你看一下识别记录，review一下这个文档。"),
        "stripped": over.pop("stripped", "review一下这个文档。"),
        "would_emit_if_enabled": over.pop("would_emit_if_enabled", True),
        "spk_emit": True,
        "shadow_emitted": True,
    }
    rec.update(over)
    obs = CtxObserver(root, sr=SR)
    a = np.zeros(SR // 2, dtype=np.float32)
    obs.submit(rec, a, a)
    obs.close()
    return rec


class TestSelection:
    def test_only_would_apply_is_a_candidate(self, tmp_path):
        for stage in ("not_located", "rejected", "unchanged", "no_prev", "would_apply"):
            _write(tmp_path / stage, stage=stage)
        picked = {
            stage: [r for r in load_records(tmp_path / stage) if is_candidate(r, tmp_path / stage)]
            for stage in ("not_located", "rejected", "unchanged", "no_prev", "would_apply")
        }
        assert len(picked["would_apply"]) == 1
        for stage in ("not_located", "rejected", "unchanged", "no_prev"):
            assert picked[stage] == [], f"{stage} 不该进分母"

    def test_excluded_when_would_not_emit(self, tmp_path):
        """启用后也不会发出去的段，用户根本看不到，不该占分母。"""
        _write(tmp_path, would_emit_if_enabled=False)
        recs = load_records(tmp_path)
        assert [r for r in recs if is_candidate(r, tmp_path)] == []

    def test_excluded_when_audio_truncated(self, tmp_path):
        import soundfile as sf

        rec = _write(tmp_path)
        recs = load_records(tmp_path)
        assert is_candidate(recs[0], tmp_path)
        sf.write(tmp_path / recs[0]["wav_ctx"], np.zeros(10, dtype=np.float32), SR, subtype="FLOAT")
        assert not is_candidate(load_records(tmp_path)[0], tmp_path)


class TestReclassify:
    """闸门改了要对全部记录重新分类——用存下来的 full/prev_text/plain，零解码。"""

    def test_reproduces_would_apply(self, tmp_path):
        rec = _write(tmp_path)
        assert reclassify(rec) == "would_apply"

    def test_rejected_becomes_would_apply_when_gate_loosens(self, monkeypatch):
        """放宽闸门会让旧的 rejected 变成新的 would_apply——正是最需要补听审的那批。

        这条是 P0-b「改了闸门必须对全部记录重跑重分类」的核心理由：只在旧的
        would_apply 上验证新闸门，会完全漏掉新参数额外放行的样本。
        """
        import ctx_prefix

        rec = {
            "plain": "嗯。",
            "prev_text": "你看一下识别记录。",
            "full": "你看一下识别记录，嗯嗯嗯这个不错。",
        }
        # 当前长度比上限 1.6，8/2=4.0 → 拒
        assert reclassify(rec) == "rejected"
        # 放宽到 5.0 → 同一条记录变成候选，且此前从未被听审过
        monkeypatch.setattr(ctx_prefix, "LEN_RATIO_MAX", 5.0)
        assert reclassify(rec) == "would_apply"

    def test_not_located_when_prefix_absent(self):
        rec = {"plain": "x", "prev_text": "完全不相干的一句话。", "full": "另一段毫无关系的话。"}
        assert reclassify(rec) == "not_located"

    def test_unchanged_when_result_equals_plain(self):
        rec = {
            "plain": "review一下这个文档。",
            "prev_text": "你看一下识别记录。",
            "full": "你看一下识别记录，review一下这个文档。",
        }
        assert reclassify(rec) == "unchanged"


class TestAggregate:
    def test_worst_of_rounds_wins(self):
        """3 轮里有 1 轮判坏，该样本即计坏——多数表决会把 1/3 概率的坏结果洗掉。"""
        v = aggregate({"s1": ["good", "good", "bad"]})
        assert v.bad == 1 and v.good == 0

    def test_non_rewrite_rounds_count_neither_way(self):
        v = aggregate({"s1": ["good", "skip", "skip"]})
        assert v.good == 1 and v.bad == 0 and v.valid == 1

    def test_sample_invalid_when_no_round_rewrote(self):
        """一轮都没改写 = 这套闸门实际不会动它，移出分母。"""
        v = aggregate({"s1": ["skip", "skip", "skip"]})
        assert v.valid == 0 and v.good == 0 and v.bad == 0

    def test_neutral_is_valid_but_neither(self):
        v = aggregate({"s1": ["neutral", "neutral", "neutral"]})
        assert v.valid == 1 and v.good == 0 and v.bad == 0


class TestPassLine:
    def _labels(self, n_good, n_neutral, n_bad=0):
        d = {}
        for i in range(n_good):
            d[f"g{i}"] = ["good"]
        for i in range(n_neutral):
            d[f"n{i}"] = ["neutral"]
        for i in range(n_bad):
            d[f"b{i}"] = ["bad"]
        return d

    def test_passes_when_all_three_met(self):
        assert aggregate(self._labels(6, 54)).passed

    def test_fails_on_any_bad(self):
        assert not aggregate(self._labels(6, 53, n_bad=1)).passed

    def test_fails_when_too_few_valid_even_with_zero_bad(self):
        """只看「0 坏、≥5 好」，有效样本缩水到十几条也会通过——所以有效数是硬条件。"""
        v = aggregate(self._labels(6, 10))
        assert v.bad == 0 and v.good >= 5 and not v.passed

    def test_fails_when_too_few_good(self):
        assert not aggregate(self._labels(2, 70)).passed
