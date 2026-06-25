"""英语发音评测 HTTP 服务(`:8098 POST /assess`)。

契约见 `docs/pronunciation-assess-api.md`,与 toolkit `english-shadow-gop-design.md` §4 对齐。
照 audio-cleanup 的并发/超时骨架,但**模型常驻进程**(发音评测要 ~1-2s 交互延迟,逐请求重载
wav2vec2 太慢) —— 用全局 Semaphore(1) 把 GPU 推理串行化,等待计数超过 QUEUE_MAX 立即 503。

传输分层(见 toolkit 设计 §3):**外部桌面端 → toolkit-server 仍是 raw body + query**;
**只有 toolkit-server → 本服务 :8098** 才是 multipart。本服务只认 multipart。
"""
import asyncio
import os
import shutil
import tempfile

from aiohttp import WSMsgType, web

import gop
import streaming

# ---- 限额与超时:具名常量(可被同名 env 覆盖),禁止散落 magic number ----
CLIENT_MAX_SIZE = int(os.environ.get("GOP_CLIENT_MAX_SIZE", str(64 * 1024 * 1024)))
QUEUE_MAX = int(os.environ.get("GOP_QUEUE_MAX", "8"))
# 单句评测目标 < 2s;给 60s 兜底(与 toolkit gop.rs 的 ASSESS_TIMEOUT 一致)。
PROCESS_TIMEOUT_SEC = float(os.environ.get("GOP_PROCESS_TIMEOUT_SEC", "60"))
PORT = int(os.environ.get("GOP_PORT", "8098"))

# 单 worker 串行(GPU)+ 等待计数。
_sem = asyncio.Semaphore(1)
_waiting = 0


def _err(status: int, message: str) -> web.Response:
    return web.json_response({"error": message}, status=status)


async def _run_assess(audio_path: str, ref_text: str, opts: dict) -> dict:
    """在线程池跑阻塞的 GOP 评测(torch 推理),不阻塞事件循环。

    单独抽出便于 test_app.py monkeypatch(避免拉 torch)。
    """
    loop = asyncio.get_event_loop()
    return await loop.run_in_executor(None, gop.assess, audio_path, ref_text, opts)


async def handle_assess(request: web.Request) -> web.Response:
    if not request.content_type.startswith("multipart/"):
        return _err(400, "Content-Type must be multipart/form-data")
    try:
        reader = await request.multipart()
    except Exception as exc:  # noqa: BLE001
        return _err(400, f"multipart parse failed: {exc}")

    tmpdir = tempfile.mkdtemp(prefix="assess-")
    try:
        return await _handle_in_tmpdir(reader, tmpdir)
    finally:
        # body 已读进内存(web.Response 持副本),安全删整个临时目录,避免泄漏。
        shutil.rmtree(tmpdir, ignore_errors=True)


async def _handle_in_tmpdir(reader, tmpdir: str) -> web.Response:
    global _waiting

    input_path = os.path.join(tmpdir, "in")
    form = {}
    has_audio = False
    async for part in reader:
        if part.name == "audio":
            has_audio = True
            with open(input_path, "wb") as f:
                while True:
                    chunk = await part.read_chunk()
                    if not chunk:
                        break
                    f.write(chunk)
        else:
            form[part.name] = (await part.read()).decode("utf-8", "ignore")

    if not has_audio:
        return _err(400, "missing 'audio' field")
    ref_text = (form.get("ref_text") or "").strip()
    if not ref_text:
        return _err(400, "missing 'ref_text' field")

    granularity = (form.get("granularity") or "word").strip().lower()
    if granularity not in ("sentence", "word"):
        return _err(400, "granularity must be sentence|word")
    opts = {"granularity": granularity, "lang": (form.get("lang") or "en").strip().lower()}

    # ---- 并发控制:等待计数 + 单 worker ----
    if _waiting >= QUEUE_MAX:
        return _err(503, "busy")
    _waiting += 1
    try:
        await _sem.acquire()
    finally:
        _waiting -= 1
    try:
        result = await asyncio.wait_for(
            _run_assess(input_path, ref_text, opts), PROCESS_TIMEOUT_SEC
        )
    except asyncio.TimeoutError:
        return _err(504, f"assessment exceeded {PROCESS_TIMEOUT_SEC:.0f}s")
    except Exception as exc:  # noqa: BLE001
        return _err(500, f"assess failed: {exc}")
    finally:
        _sem.release()
    return web.json_response(result)


async def handle_assess_stream(request: web.Request) -> web.WebSocketResponse:
    """流式发音评测 WS(契约见 docs/pronunciation-assess-api.md「/assess/stream」)。

    上行:hello(JSON,首帧 ref_text/granularity)→ 二进制 PCM(s16le 16k 单声道)块 → end(JSON)。
    下行:ready → partial(逐词落定,可含 phones)… → final(批量 GOP 权威分)。
    GPU 推理用 run_in_executor + 复用 `_sem` 串行(与批量 /assess 共用一把锁,逐次 acquire)。
    """
    ws = web.WebSocketResponse(max_msg_size=8 * 1024 * 1024)
    await ws.prepare(request)
    loop = asyncio.get_event_loop()
    assessor = None

    async def infer(fn, *a):
        async with _sem:
            return await loop.run_in_executor(None, fn, *a)

    try:
        async for msg in ws:
            if msg.type == WSMsgType.TEXT:
                try:
                    d = __import__("json").loads(msg.data)
                except ValueError:
                    await ws.send_json({"type": "error", "message": "bad json"}); continue
                t = d.get("type")
                if t == "hello":
                    ref = (d.get("ref_text") or "").strip()
                    if not ref:
                        await ws.send_json({"type": "error", "message": "missing ref_text"}); break
                    gran = (d.get("granularity") or "word").strip().lower()
                    assessor = streaming.StreamingAssessor(ref, gran)
                    await ws.send_json({"type": "ready"})
                elif t == "end":
                    if assessor is None:
                        await ws.send_json({"type": "error", "message": "no hello"}); break
                    final = await infer(assessor.finish)
                    await ws.send_json({"type": "final", **final})
                    await ws.close(); break
                else:
                    await ws.send_json({"type": "error", "message": f"unknown type {t}"})
            elif msg.type == WSMsgType.BINARY:
                if assessor is None:
                    await ws.send_json({"type": "error", "message": "no hello before audio"}); break
                try:
                    updates = await infer(assessor.push, bytes(msg.data))
                except Exception as exc:  # noqa: BLE001
                    await ws.send_json({"type": "error", "message": f"push failed: {exc}"}); break
                for u in updates:
                    await ws.send_json({"type": "partial", **u})
            elif msg.type == WSMsgType.ERROR:
                break
    finally:
        if not ws.closed:
            await ws.close()
    return ws


async def handle_health(_request: web.Request) -> web.Response:
    gpu = False
    try:
        import torch
        gpu = bool(torch.cuda.is_available())
    except Exception:  # noqa: BLE001
        gpu = False
    return web.json_response({
        "model_loaded": gop._model is not None,  # 常驻模型;首个请求后置 true
        "model_id": gop.MODEL_ID,
        "gpu": gpu,
    })


def make_app() -> web.Application:
    app = web.Application(client_max_size=CLIENT_MAX_SIZE)
    app.router.add_post("/assess", handle_assess)
    app.router.add_get("/assess/stream", handle_assess_stream)
    app.router.add_get("/health", handle_health)
    return app


if __name__ == "__main__":
    web.run_app(make_app(), host="0.0.0.0", port=PORT)
