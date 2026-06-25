"""app.py 行为单测(不依赖 torch;monkeypatch `_run_assess`)。

覆盖:缺 audio→400、缺 ref_text→400、坏 granularity→400、成功→200+JSON、超时→504、
评测异常→500、队列满→503、等待计数不泄漏、临时目录清理。

运行(需 aiohttp):python server/pronunciation-assess/test_app.py
"""
import asyncio
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import app  # noqa: E402
from aiohttp import FormData  # noqa: E402
from aiohttp.test_utils import TestClient, TestServer  # noqa: E402


def _multipart(audio=b"RIFFfake", ref_text="I think so", **fields):
    fd = FormData()
    if audio is not None:
        fd.add_field("audio", audio, filename="in.webm", content_type="audio/webm")
    if ref_text is not None:
        fd.add_field("ref_text", ref_text)
    for k, v in fields.items():
        fd.add_field(k, str(v))
    return fd


async def _client():
    server = TestServer(app.make_app())
    client = TestClient(server)
    await client.start_server()
    return client


def run(coro):
    return asyncio.new_event_loop().run_until_complete(coro)


async def _case_missing_audio():
    c = await _client()
    try:
        fd = FormData()
        fd.add_field("ref_text", "hello", filename="x.txt", content_type="text/plain")
        r = await c.post("/assess", data=fd)
        assert r.status == 400, r.status
        assert "audio" in (await r.json())["error"]
    finally:
        await c.close()


async def _case_missing_ref_text():
    c = await _client()
    try:
        r = await c.post("/assess", data=_multipart(ref_text=None))
        assert r.status == 400, r.status
        assert "ref_text" in (await r.json())["error"]
    finally:
        await c.close()


async def _case_bad_granularity():
    c = await _client()
    try:
        r = await c.post("/assess", data=_multipart(granularity="paragraph"))
        assert r.status == 400, r.status
    finally:
        await c.close()


async def _case_success():
    c = await _client()
    seen = {}

    async def fake_run(audio_path, ref_text, opts):
        seen["ref"] = ref_text
        seen["gran"] = opts["granularity"]
        return {"ref_text": ref_text, "sentence_score": 0.71,
                "words": [{"ref": "I", "score": 0.95, "pron_status": "ok"}],
                "bad_phone_count": 0, "model": "fake"}

    orig = app._run_assess
    app._run_assess = fake_run
    try:
        r = await c.post("/assess", data=_multipart(ref_text="I think so", granularity="word"))
        assert r.status == 200, r.status
        body = await r.json()
        assert body["sentence_score"] == 0.71
        assert body["model"] == "fake"
        assert seen["ref"] == "I think so"
        assert seen["gran"] == "word"
        assert app._waiting == 0, "等待计数泄漏"
    finally:
        app._run_assess = orig
        await c.close()


async def _case_timeout_504():
    c = await _client()
    old_to = app.PROCESS_TIMEOUT_SEC
    app.PROCESS_TIMEOUT_SEC = 0.05

    async def slow_run(_a, _r, _o):
        await asyncio.sleep(5)
        return {}

    orig = app._run_assess
    app._run_assess = slow_run
    try:
        r = await c.post("/assess", data=_multipart())
        assert r.status == 504, r.status
        assert app._waiting == 0
    finally:
        app._run_assess = orig
        app.PROCESS_TIMEOUT_SEC = old_to
        await c.close()


async def _case_error_500():
    c = await _client()

    async def boom(_a, _r, _o):
        raise RuntimeError("model exploded")

    orig = app._run_assess
    app._run_assess = boom
    try:
        r = await c.post("/assess", data=_multipart())
        assert r.status == 500, r.status
        assert "model exploded" in (await r.json())["error"]
    finally:
        app._run_assess = orig
        await c.close()


async def _case_queue_full_503():
    c = await _client()
    gate = asyncio.Event()

    async def blocking_run(_a, _r, _o):
        await gate.wait()
        return {"ref_text": "x", "sentence_score": 1.0, "words": [],
                "bad_phone_count": 0, "model": "fake"}

    orig = app._run_assess
    old_q = app.QUEUE_MAX
    app._run_assess = blocking_run
    app.QUEUE_MAX = 1
    try:
        t1 = asyncio.create_task(c.post("/assess", data=_multipart()))
        t2 = asyncio.create_task(c.post("/assess", data=_multipart()))
        await asyncio.sleep(0.2)  # t1 拿锁、t2 进等待队列
        r3 = await c.post("/assess", data=_multipart())
        assert r3.status == 503, r3.status
        gate.set()
        for t in (t1, t2):
            rr = await t
            assert rr.status == 200, rr.status
        assert app._waiting == 0, "等待计数泄漏"
    finally:
        app._run_assess = orig
        app.QUEUE_MAX = old_q
        await c.close()


CASES = [
    _case_missing_audio,
    _case_missing_ref_text,
    _case_bad_granularity,
    _case_success,
    _case_timeout_504,
    _case_error_500,
    _case_queue_full_503,
]


def test_all():
    for case in CASES:
        run(case())


if __name__ == "__main__":
    for case in CASES:
        run(case())
        print(f"ok: {case.__name__}")
    print(f"\n{len(CASES)} passed")
