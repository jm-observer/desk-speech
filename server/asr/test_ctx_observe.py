"""前缀上下文解码观测落盘的测试。

这些性质不是凭空要求的，每一条都对应上一版的一个具体故障或一个具体代码约束，
详见 docs/asr-ctx-prefix-postmortem.md：

- 样本要么完整要么不存在——只用「文件存在」当判据不够，崩溃会留下存在但截断的 wav；
- 队列满宁可丢样本，也绝不反压识别路径（写盘卡住会阻塞 asyncio 事件循环）；
- 容量超限停写而不是覆盖旧数据——旧数据才是评测语料。
"""

import json

import numpy as np
import soundfile as sf

from ctx_observe import CtxObserver, sample_is_complete

SR = 16000


def _rec(key="s1-000100-000900", stage="would_apply"):
    return {"key": key, "stage": stage, "plain": "伪略一下这个文档。"}


def _audio(secs):
    return np.linspace(-0.5, 0.5, int(SR * secs), dtype=np.float32)


class TestHappyPath:
    def test_writes_jsonl_and_two_wavs(self, tmp_path):
        obs = CtxObserver(tmp_path, sr=SR)
        assert obs.submit(_rec(), _audio(1.5), _audio(0.6))
        obs.close()

        lines = list(tmp_path.rglob("*.jsonl"))
        assert len(lines) == 1
        rec = json.loads(lines[0].read_text(encoding="utf-8").strip())
        assert rec["stage"] == "would_apply"
        # 帧数必须记进 JSON——评测侧靠它校验 wav 没被截断
        assert rec["ctx_frames"] == int(SR * 1.5)
        assert rec["plain_frames"] == int(SR * 0.6)
        assert sample_is_complete(rec, tmp_path)

    def test_audio_roundtrips_without_quantisation(self, tmp_path):
        """必须是 float32，存 PCM16 会引入量化差异，那就不是「生产当时真正的输入」。"""
        obs = CtxObserver(tmp_path, sr=SR)
        a = _audio(0.4)
        obs.submit(_rec(), a, a)
        obs.close()
        rec = json.loads(next(tmp_path.rglob("*.jsonl")).read_text(encoding="utf-8").strip())
        back, sr = sf.read(tmp_path / rec["wav_ctx"], dtype="float32")
        assert sr == SR
        np.testing.assert_array_equal(back, a)

    def test_files_are_day_partitioned(self, tmp_path):
        obs = CtxObserver(tmp_path, sr=SR)
        obs.submit(_rec(), _audio(0.3), _audio(0.3))
        obs.close()
        day_dir = next(tmp_path.iterdir())
        assert len(day_dir.name) == 10 and day_dir.name.count("-") == 2  # YYYY-MM-DD


class TestAtomicity:
    def test_no_jsonl_line_when_audio_write_fails(self, tmp_path, monkeypatch):
        """写盘中途失败时不能留下「有行但音频残缺」的样本。"""
        obs = CtxObserver(tmp_path, sr=SR)
        calls = {"n": 0}

        def boom(*a, **kw):
            calls["n"] += 1
            if calls["n"] == 2:  # 第二个 wav 失败
                raise OSError("disk gone")
            return sf.write(*a, **kw)

        monkeypatch.setattr("ctx_observe.sf.write", boom)
        obs.submit(_rec(), _audio(0.3), _audio(0.3))
        obs.close()

        assert not list(tmp_path.rglob("*.jsonl")) or not next(
            tmp_path.rglob("*.jsonl")
        ).read_text(encoding="utf-8").strip()
        assert obs.stats()["write_errors"] == 1

    def test_no_temp_files_left_behind(self, tmp_path, monkeypatch):
        obs = CtxObserver(tmp_path, sr=SR)
        monkeypatch.setattr(
            "ctx_observe.sf.write", lambda *a, **kw: (_ for _ in ()).throw(OSError("x"))
        )
        obs.submit(_rec(), _audio(0.3), _audio(0.3))
        obs.close()
        assert not list(tmp_path.rglob("*.tmp"))


class TestCompleteness:
    def test_truncated_wav_is_not_complete(self, tmp_path):
        """只检查「文件存在」不够——截断的 wav 照样存在。"""
        obs = CtxObserver(tmp_path, sr=SR)
        obs.submit(_rec(), _audio(1.0), _audio(0.5))
        obs.close()
        rec = json.loads(next(tmp_path.rglob("*.jsonl")).read_text(encoding="utf-8").strip())
        assert sample_is_complete(rec, tmp_path)

        sf.write(tmp_path / rec["wav_ctx"], _audio(0.2), SR, subtype="FLOAT")
        assert not sample_is_complete(rec, tmp_path)

    def test_missing_wav_is_not_complete(self, tmp_path):
        obs = CtxObserver(tmp_path, sr=SR)
        obs.submit(_rec(), _audio(0.5), _audio(0.5))
        obs.close()
        rec = json.loads(next(tmp_path.rglob("*.jsonl")).read_text(encoding="utf-8").strip())
        (tmp_path / rec["wav_plain"]).unlink()
        assert not sample_is_complete(rec, tmp_path)


class TestBackpressure:
    def test_queue_full_drops_instead_of_blocking(self, tmp_path):
        """识别路径绝不能被写盘拖住：队列满就丢，计数，立即返回。"""
        obs = CtxObserver(tmp_path, sr=SR, queue_max=1, _start_worker=False)
        accepted = [obs.submit(_rec(f"k{i}"), _audio(0.2), _audio(0.2)) for i in range(5)]
        assert accepted[0] is True
        assert accepted.count(False) == 4
        assert obs.stats()["dropped"] == 4

    def test_submit_never_raises(self, tmp_path):
        obs = CtxObserver(tmp_path, sr=SR)
        # 音频为 None（例如 stage=disabled 那类根本没产生音频的记录）不应抛
        assert obs.submit(_rec(stage="no_prev"), None, None)
        obs.close()
        rec = json.loads(next(tmp_path.rglob("*.jsonl")).read_text(encoding="utf-8").strip())
        assert rec["wav_ctx"] is None and rec["ctx_frames"] is None


class TestCapacity:
    def test_stops_writing_when_over_budget(self, tmp_path):
        """超限停写，而不是覆盖旧数据——旧数据才是评测语料。"""
        obs = CtxObserver(tmp_path, sr=SR, max_bytes=50_000)
        for i in range(6):
            obs.submit(_rec(f"k{i}"), _audio(0.5), _audio(0.5))
        obs.close()
        assert obs.stats()["over_budget"] > 0
        written = len(
            [ln for f in tmp_path.rglob("*.jsonl") for ln in f.read_text(encoding="utf-8").splitlines() if ln]
        )
        assert 0 < written < 6


class TestRecordContract:
    """钉死 app.py::_observe_ctx 实际产出的记录形状。

    app.py 本地导不进来（funasr 是容器依赖），所以这里照它的字段构造一条，
    确保观测层能原样落盘并通过完整性校验。字段名改动会在这里先炸。
    """

    def test_full_record_roundtrips(self, tmp_path):
        rec = {
            "key": "ab12cd34ef56-00007",
            "shadow": True,
            "plain": "伪略一下这个文档。",
            "dur_ms": 2410,
            "prev_text": "你看一下识别记录。",
            "prev_dur_ms": 2840,
            "gap_ms": 250,
            "full": "你看一下识别记录，review一下这个文档。",
            "stripped": "review一下这个文档。",
            "stage": "would_apply",
            "t_start_ms": 12000,
            "t_end_ms": 14410,
            "spk_emit": True,
            "shadow_emitted": True,
            "would_emit_if_enabled": True,
        }
        obs = CtxObserver(tmp_path, sr=SR)
        assert obs.submit(rec, _audio(5.5), _audio(2.41))
        obs.close()

        back = json.loads(next(tmp_path.rglob("*.jsonl")).read_text(encoding="utf-8").strip())
        for k in rec:
            assert k in back, f"字段 {k} 丢了"
        assert sample_is_complete(back, tmp_path)
        # 评测入选的三个判据都要能从记录里读出来
        assert back["stage"] == "would_apply"
        assert back["would_emit_if_enabled"] is True

    def test_stage_without_audio_is_complete(self, tmp_path):
        """disabled / no_prev 这类根本没跑重解的记录也要能落盘并算完整。"""
        obs = CtxObserver(tmp_path, sr=SR)
        obs.submit({"key": "k", "stage": "no_prev", "plain": "x", "full": None}, None, None)
        obs.close()
        back = json.loads(next(tmp_path.rglob("*.jsonl")).read_text(encoding="utf-8").strip())
        assert sample_is_complete(back, tmp_path)
