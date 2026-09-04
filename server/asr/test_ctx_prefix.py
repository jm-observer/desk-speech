"""前缀上下文解码的纯逻辑测试。

背景（toolkit 段 #13340）：说的是「review 一下这个文档」，孤立解码得到
「伪略一下这个文档。」。同一段音频前面拼上一整句无关中文后重解，得到
「你看一下识别记录，review一下这个文档。」——完全正确。

难点不在拼音频，在**把前缀那部分文本切掉**：识别接口只返回整段字符串，
Whisper 侧还显式 without_timestamps，没有词级时间戳可用。这里的做法是拿
「上一句已经识别出来的文本」去对齐切分，因此可以纯文本测试。
"""

import pytest

from ctx_prefix import accept_ctx_text, strip_known_prefix


class TestStripKnownPrefix:
    def test_real_case_13340(self):
        """实测数据：前缀是 #13341，本句是 #13340。"""
        got = strip_known_prefix(
            "你看一下识别记录，review一下这个文档。", "你看一下识别记录。"
        )
        assert got == "review一下这个文档。"

    def test_prefix_redecoded_slightly_differently(self):
        """前缀重解码时字词会有出入（标点/口语词），仍须能定位。

        这是常态而非例外：同一段音频跟别的音频拼在一起解码，前缀部分的
        输出本来就不保证与上次逐字相同。
        """
        got = strip_known_prefix(
            "你看一下这个识别记录 review 一下这个文档。", "你看一下识别记录。"
        )
        assert got is not None
        assert "review" in got
        assert "识别记录" not in got

    def test_returns_none_when_prefix_absent(self):
        """定位不到前缀就必须放弃，绝不能瞎切——切错正文比不切更糟。"""
        assert strip_known_prefix("完全不相干的一句话。", "你看一下识别记录。") is None

    def test_does_not_overcut_when_current_repeats_prefix_words(self):
        """本句开头重复了前缀里的词，不能把本句一起吃掉。"""
        got = strip_known_prefix(
            "你看一下识别记录，识别记录里面有问题。", "你看一下识别记录。"
        )
        assert got == "识别记录里面有问题。"

    def test_returns_none_when_only_part_of_prefix_shows_up(self):
        """前缀音频与 prev_text 对不上时必须放弃。

        这正是 app.py 里 CTX_PREFIX_MAX_PREV_MS 存在的理由：若把上一句的音频
        截成尾部若干秒当前缀，解出来的只会是尾部那点内容，而 prev_text 是整句
        的文本，覆盖率必然不足。这里钉住「不足就返回 None」，让那条路径只会
        退回无上下文的结果，而不是切错正文。
        """
        prev = "我先说一段很长的话把上下文铺开然后再讲别的内容"
        got = strip_known_prefix("再讲别的内容，review一下这个文档。", prev)
        assert got is None

    def test_empty_inputs(self):
        assert strip_known_prefix("", "上一句") is None
        assert strip_known_prefix("整段文本", "") is None


class TestAcceptCtxText:
    def test_accepts_comparable_length(self):
        assert accept_ctx_text("review一下这个文档。", "伪略一下这个文档。")

    def test_rejects_empty(self):
        assert not accept_ctx_text("", "伪略一下这个文档。")
        assert not accept_ctx_text("   ", "伪略一下这个文档。")

    def test_rejects_length_explosion(self):
        """前缀没切干净时结果会明显变长——这是最常见的失败形态。"""
        assert not accept_ctx_text(
            "你看一下识别记录，review一下这个文档。", "伪略一下这个文档。"
        )

    def test_rejects_length_collapse(self):
        """切过头会把正文吃掉，剩一点残渣。"""
        assert not accept_ctx_text("档。", "伪略一下这个文档。")

    def test_short_texts_are_not_judged_by_ratio_alone(self):
        """极短文本上比例判据没有意义，几个字的差异就能触发数倍变化。"""
        assert accept_ctx_text("好的", "好")
