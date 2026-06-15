"""音频清洗 HTTP 服务（Plan 2）。

契约见 docs/2026-06-14-audio-cleanup/audio-cleanup-plan-2.md 与 docs/audio-cleanup-api.md。

并发与超时（核心，防资源争用与后台泄漏）：
  - aiohttp 默认并发处理请求 —— 必须用全局 Semaphore(1) 把「整条 pipeline」串行化；
  - 另用计数器记录「等待中」请求，超过 QUEUE_MAX 立即 503，不入队；
  - pipeline 跑在「可终止子进程」(pipeline.py 的 __main__)：超时 PROCESS_TIMEOUT_SEC 即
    kill 子进程，**等其真实回收后**才释放锁、返回 504 —— 锁的持有时间 ≥ 子进程存活时间，
    杜绝 504 后旧任务仍跑、新任务又叠加。模型在子进程内加载、随退出释放，空闲 GPU 零占用。
"""
import asyncio
import json
import os
import shutil
import signal
import sys
import tempfile

from aiohttp import web

from pipeline import CleanOpts, probe_duration_sec

# ---- 限额与超时：具名常量（可被同名 env 覆盖），禁止散落 magic number ----
CLIENT_MAX_SIZE = int(os.environ.get("CLEAN_CLIENT_MAX_SIZE", str(512 * 1024 * 1024)))
MAX_DURATION_SEC = float(os.environ.get("CLEAN_MAX_DURATION_SEC", "600"))
QUEUE_MAX = int(os.environ.get("CLEAN_QUEUE_MAX", "4"))
PROCESS_TIMEOUT_SEC = float(os.environ.get("CLEAN_PROCESS_TIMEOUT_SEC", "600"))
PORT = int(os.environ.get("CLEAN_PORT", "8097"))
PIPELINE_PY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pipeline.py")

_CONTENT_TYPE = {"wav": "audio/wav", "mp3": "audio/mpeg", "flac": "audio/flac"}

# 单 worker 串行 + 等待计数。
_sem = asyncio.Semaphore(1)
_waiting = 0


def _truthy(v: str) -> bool:
    return str(v).strip().lower() in ("1", "true", "on", "yes")


def _parse_opts(form) -> CleanOpts:
    """从 multipart 字段构造 CleanOpts。缺省走 CleanOpts 默认（pause=duck, sr=48000…）。"""
    def field(name, default=None):
        v = form.get(name)
        return v if v is not None else default

    loud_raw = field("loudness")
    if loud_raw is None:
        loudness = -16.0
    elif str(loud_raw).strip().lower() == "off":
        loudness = None
    else:
        loudness = float(loud_raw)

    opts = CleanOpts(
        separate=_truthy(field("separate", "0")),
        denoise=_truthy(field("denoise", "1")),
        pause=str(field("pause", "duck")).strip().lower(),
        level=str(field("level", "balanced")).strip().lower(),
        loudness=loudness,
        sr=int(field("sr", "48000")),
        fmt=str(field("format", "wav")).strip().lower(),
    )
    opts.validate()
    return opts


def _err(status: int, message: str) -> web.Response:
    return web.json_response({"error": message}, status=status)


def _kill_process_group(proc) -> None:
    """SIGKILL 整个进程组——pipeline.py 内部会起 ffmpeg 等孙进程，只 kill 直接子进程会遗留它们，
    违背「504 后无后台叠加」。子进程以 start_new_session=True 独立成组，这里按组 kill。"""
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        proc.kill()  # 进程已退/取不到组，退回单进程 kill


async def _run_subprocess(input_path: str, output_path: str, opts: CleanOpts, meta_path: str):
    """拉起可终止子进程跑 pipeline；超时 kill 整个进程组并真实回收。返回 (ok, meta_or_errmsg)。"""
    proc = await asyncio.create_subprocess_exec(
        sys.executable, PIPELINE_PY, input_path, output_path,
        json.dumps(opts.to_dict()), meta_path,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
        start_new_session=True,  # 独立进程组，便于超时按组 kill（含 ffmpeg 孙进程）
    )
    try:
        _, stderr = await asyncio.wait_for(proc.communicate(), PROCESS_TIMEOUT_SEC)
    except asyncio.TimeoutError:
        _kill_process_group(proc)
        await proc.wait()          # 等子进程真实回收后再返回（锁随之释放）
        return False, ("timeout", "processing exceeded %ds, split the input" % PROCESS_TIMEOUT_SEC)
    except asyncio.CancelledError:
        # 客户端断开 / 反代超时 / 调用方取消 → handler 在此被取消。必须连进程组一起杀
        # （含 ffmpeg 孙进程）并真实回收，否则旧 pipeline 继续跑、sem 释放后新请求叠加。
        _kill_process_group(proc)
        try:
            await proc.wait()
        except asyncio.CancelledError:
            pass  # wait 再被取消也无妨：进程组已 SIGKILL，无后台任务残留
        raise     # 回收后再传播取消
    if proc.returncode != 0:
        return False, ("error", stderr.decode("utf-8", "ignore")[:200] or "pipeline failed")
    with open(meta_path, "r", encoding="utf-8") as f:
        return True, json.load(f)


async def handle_clean(request: web.Request) -> web.Response:
    global _waiting

    # ---- 解析（400 类错误不占队列）----
    if not request.content_type.startswith("multipart/"):
        return _err(400, "Content-Type must be multipart/form-data")
    try:
        reader = await request.multipart()
    except Exception as exc:  # noqa: BLE001
        return _err(400, f"multipart parse failed: {exc}")

    tmpdir = tempfile.mkdtemp(prefix="clean-")
    try:
        return await _handle_clean_in_tmpdir(reader, tmpdir)
    finally:
        # body 已在下面读进内存（web.Response 持有副本），可安全删整个临时目录——
        # 否则每个请求泄漏 in/out.*/meta.json，512MiB 上传长跑会吃满磁盘。
        shutil.rmtree(tmpdir, ignore_errors=True)


async def _handle_clean_in_tmpdir(reader, tmpdir: str) -> web.Response:
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

    try:
        opts = _parse_opts(form)
    except (ValueError, KeyError) as exc:
        return _err(400, f"bad option: {exc}")

    # ---- 时长早拒（解码探测，便于 422 早返回）----
    try:
        duration = probe_duration_sec(input_path)
    except RuntimeError as exc:
        return _err(400, f"decode failed: {exc}")
    if duration > MAX_DURATION_SEC:
        return _err(422, f"audio {duration:.0f}s exceeds max {MAX_DURATION_SEC:.0f}s; split it")

    # ---- 并发控制：等待计数 + 单 worker ----
    if _waiting >= QUEUE_MAX:
        return _err(503, "busy")
    # 等待计数包在 try/finally：即便等待 acquire 期间 handler 被取消（客户端断开），
    # 也要把 _waiting 减回去，否则几次取消后 _waiting>=QUEUE_MAX 会永久错误 503。
    _waiting += 1
    try:
        await _sem.acquire()
    finally:
        _waiting -= 1
    try:
        output_path = os.path.join(tmpdir, f"out.{opts.fmt}")
        meta_path = os.path.join(tmpdir, "meta.json")
        ok, payload = await _run_subprocess(input_path, output_path, opts, meta_path)
        if not ok:
            kind, msg = payload
            return _err(504 if kind == "timeout" else 500, msg)
        with open(output_path, "rb") as f:
            body = f.read()
        meta = payload
        return web.Response(
            body=body,
            content_type=_CONTENT_TYPE.get(opts.fmt, "application/octet-stream"),
            headers={
                "X-Cleanup-Stages": ",".join(meta["stages"]),
                "X-Cleanup-In-LUFS": f"{meta['in_lufs']:.1f}",
                "X-Cleanup-Out-LUFS": f"{meta['out_lufs']:.1f}",
            },
        )
    finally:
        _sem.release()


async def handle_health(_request: web.Request) -> web.Response:
    gpu = False
    try:
        import torch
        gpu = bool(torch.cuda.is_available())
    except Exception:  # noqa: BLE001
        gpu = False
    return web.json_response({
        "model_loaded": True,   # 子进程模型，按需加载；服务存活即视为就绪
        "stages_available": ["separate", "denoise", "vad", "loudness"],
        "gpu": gpu,
    })


def make_app() -> web.Application:
    app = web.Application(client_max_size=CLIENT_MAX_SIZE)
    app.router.add_post("/clean", handle_clean)
    app.router.add_get("/health", handle_health)
    return app


if __name__ == "__main__":
    web.run_app(make_app(), host="0.0.0.0", port=PORT)
