"""app.py 行为单测（不依赖 torch/demucs/DF；只需 aiohttp）。

用 monkeypatch 替换 `_run_subprocess` / `probe_duration_sec`，覆盖 Plan 里最关键的 app 行为：
缺 audio→400、超时长→422、超时→504、队列满→503、成功→200+头、临时目录清理、等待计数不泄漏。

运行（需 aiohttp，在容器/CI 里）：python -m pytest server/audio-cleanup/test_app.py
或：python server/audio-cleanup/test_app.py
"""
import asyncio
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import app  # noqa: E402
from aiohttp.test_utils import TestClient, TestServer  # noqa: E402


def _multipart(audio=b"RIFFfake", **fields):
    """构造一个最小 multipart body + headers。"""
    import io
    from aiohttp import FormData

    fd = FormData()
    fd.add_field("audio", audio, filename="in.wav", content_type="audio/wav")
    for k, v in fields.items():
        fd.add_field(k, str(v))
    # FormData -> payload；交给 TestClient.post 用 data= 即可，这里仅返回 fd
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
        from aiohttp import FormData

        fd = FormData()
        # 用一个带 filename 的非 audio 字段强制 multipart（纯文本字段会被 aiohttp 编成
        # urlencoded，走不到「缺 audio」分支）。这样 handler 进 multipart、循环找不到 audio。
        fd.add_field("notaudio", b"x", filename="x.txt", content_type="text/plain")
        r = await c.post("/clean", data=fd)
        assert r.status == 400, r.status
        assert "audio" in (await r.json())["error"]
    finally:
        await c.close()


async def _case_duration_422(monkeypatch_max=1.0):
    c = await _client()
    orig = app.probe_duration_sec
    app.probe_duration_sec = lambda _p: 9999.0
    old_max = app.MAX_DURATION_SEC
    app.MAX_DURATION_SEC = monkeypatch_max
    try:
        r = await c.post("/clean", data=_multipart())
        assert r.status == 422, r.status
    finally:
        app.probe_duration_sec = orig
        app.MAX_DURATION_SEC = old_max
        await c.close()


async def _case_timeout_504():
    c = await _client()
    app.probe_duration_sec = lambda _p: 1.0

    async def fake_run(_inp, _out, _opts, _meta):
        return False, ("timeout", "processing exceeded 600s, split the input")

    orig = app._run_subprocess
    app._run_subprocess = fake_run
    try:
        r = await c.post("/clean", data=_multipart())
        assert r.status == 504, r.status
    finally:
        app._run_subprocess = orig
        await c.close()


async def _case_success_and_tmp_cleanup():
    c = await _client()
    app.probe_duration_sec = lambda _p: 1.0
    seen = {}

    async def fake_run(_inp, output_path, _opts, _meta):
        seen["tmpdir"] = os.path.dirname(output_path)
        with open(output_path, "wb") as f:
            f.write(b"CLEANWAV")
        return True, {"stages": ["decode", "encode"], "in_lufs": -20.0, "out_lufs": -16.0}

    orig = app._run_subprocess
    app._run_subprocess = fake_run
    try:
        r = await c.post("/clean", data=_multipart())
        assert r.status == 200, r.status
        assert await r.read() == b"CLEANWAV"
        assert r.headers["X-Cleanup-Stages"] == "decode,encode"
        assert r.headers["X-Cleanup-Out-LUFS"] == "-16.0"
        # 临时目录在响应返回后必须已删除（无泄漏）。
        assert not os.path.exists(seen["tmpdir"]), "tmpdir 未清理"
        assert app._waiting == 0, "等待计数泄漏"
    finally:
        app._run_subprocess = orig
        await c.close()


async def _case_queue_full_503():
    c = await _client()
    app.probe_duration_sec = lambda _p: 1.0
    gate = asyncio.Event()

    async def blocking_run(_inp, output_path, _opts, _meta):
        await gate.wait()  # 占住 worker，让后续请求堆在等待队列
        with open(output_path, "wb") as f:
            f.write(b"x")
        return True, {"stages": ["decode"], "in_lufs": 0.0, "out_lufs": 0.0}

    orig = app._run_subprocess
    old_qmax = app.QUEUE_MAX
    app._run_subprocess = blocking_run
    app.QUEUE_MAX = 1
    try:
        # 1 个在跑 + 1 个在等待队列（QUEUE_MAX=1）→ 第 3 个应立即 503。
        t1 = asyncio.create_task(c.post("/clean", data=_multipart()))
        t2 = asyncio.create_task(c.post("/clean", data=_multipart()))
        await asyncio.sleep(0.2)  # 让 t1 拿锁、t2 进等待
        r3 = await c.post("/clean", data=_multipart())
        assert r3.status == 503, r3.status
        gate.set()
        for t in (t1, t2):
            rr = await t
            assert rr.status == 200, rr.status
        assert app._waiting == 0, "等待计数泄漏"
    finally:
        app._run_subprocess = orig
        app.QUEUE_MAX = old_qmax
        await c.close()


async def _case_cancel_kills_subprocess():
    """取消正在跑的 _run_subprocess 时，必须杀进程组并回收子进程（防后台叠加）。"""
    killed = {"n": 0}

    class FakeProc:
        pid = 999999

        async def communicate(self):
            await asyncio.Event().wait()  # 永不返回，模拟长跑 pipeline

        async def wait(self):
            return 0

    orig_create = asyncio.create_subprocess_exec
    orig_kill = app._kill_process_group

    async def fake_create(*_a, **_k):
        return FakeProc()

    app._kill_process_group = lambda _proc: killed.__setitem__("n", killed["n"] + 1)
    asyncio.create_subprocess_exec = fake_create
    try:
        from pipeline import CleanOpts

        task = asyncio.ensure_future(
            app._run_subprocess("in", "out", CleanOpts(), "meta")
        )
        await asyncio.sleep(0.1)  # 让它进到 await communicate()
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass
        assert killed["n"] == 1, f"取消时未杀进程组 (n={killed['n']})"
    finally:
        asyncio.create_subprocess_exec = orig_create
        app._kill_process_group = orig_kill


CASES = [
    _case_missing_audio,
    _case_duration_422,
    _case_timeout_504,
    _case_success_and_tmp_cleanup,
    _case_queue_full_503,
    _case_cancel_kills_subprocess,
]


def test_all():
    for case in CASES:
        run(case())


if __name__ == "__main__":
    for case in CASES:
        run(case())
        print(f"ok: {case.__name__}")
    print(f"\n{len(CASES)} passed")
