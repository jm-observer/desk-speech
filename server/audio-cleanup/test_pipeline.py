"""纯逻辑单测（不依赖 torch/demucs/DF/aiohttp）：CleanOpts 校验与序列化、opts 解析。

stage 函数与端到端清洗需要 ML 依赖，在 GB10 上跑冒烟（见 README）；本文件只覆盖可在
任意机器跑的纯逻辑，保证 CI/开发机也能验证契约边界。
运行：  python -m pytest server/audio-cleanup/test_pipeline.py
或：    python server/audio-cleanup/test_pipeline.py   （内置断言，无 pytest 也能跑）
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from pipeline import CleanOpts  # noqa: E402


def test_defaults_match_design():
    # 设计拍板：pause 默认 duck、denoise 默认开、sr 默认 48000、loudness 默认 -16。
    o = CleanOpts()
    assert o.pause == "duck"
    assert o.denoise is True
    assert o.separate is False
    assert o.sr == 48000
    assert o.loudness == -16.0
    o.validate()


def test_roundtrip():
    o = CleanOpts(separate=True, pause="off", sr=16000, fmt="flac", loudness=None)
    assert CleanOpts.from_dict(o.to_dict()) == o


def test_validate_rejects_bad_values():
    for bad in (
        CleanOpts(pause="nope"),
        CleanOpts(level="extreme"),
        CleanOpts(fmt="ogg"),
        CleanOpts(sr=44100),
    ):
        try:
            bad.validate()
        except ValueError:
            continue
        raise AssertionError(f"validate should reject {bad}")


def test_loudness_off_is_none():
    # loudness=None 表示关闭归一化（管线据此跳过 stage_loudness）。
    o = CleanOpts(loudness=None)
    o.validate()
    assert o.loudness is None


def _run_all():
    fns = [v for k, v in globals().items() if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"ok: {fn.__name__}")
    print(f"\n{len(fns)} passed")


if __name__ == "__main__":
    _run_all()
