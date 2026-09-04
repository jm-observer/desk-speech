"""前缀上下文解码的观测落盘：队列 + 后台线程 + 原子写。

## 为什么要单独做这么一层

上一版（2026-09-04 上线即回滚）只往 stdout 打了四行日志，而且**改写与拒绝这两条
——唯二会影响输出的分支——记得最少**：只有 `plain` 和结果，没有 `full`、没有
`prev_text`、没有音频。关停时容器一重建，全部原始数据随之丢失。结果是事故之后
既判断不了那 8 次改写是好是坏，也归因不了「样，」「部去」到底是前缀残留还是模型幻觉。

复盘与完整的性质清单见 `docs/asr-ctx-prefix-postmortem.md`。这里实现其中的观测部分。

## 三条硬性质

1. **绝不反压识别路径**。调用点是同步函数、跑在 asyncio 的 `finalize` 里；直接同步
   写盘遇到慢盘/挂载卡死**不会抛异常，而是阻塞事件循环**，拖慢甚至冻结整个服务。
   所以 `submit()` 只入队、永不阻塞、永不抛；队列满就丢样本并计数。
2. **样本要么完整要么不存在**。一条记录 = 一行 JSON + 两个 wav。先写 wav 临时文件、
   fsync、原子改名，最后才追加 JSONL 行并 fsync——**JSONL 行是「该样本完整」的提交标记**。
   中途失败就把临时文件和已改名的 wav 都删掉。
3. **超容量停写，不覆盖旧数据**。旧数据才是评测语料。
"""

import json
import os
import queue
import threading
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
import soundfile as sf

# 队列容量：够吸收几秒的突发即可。满了宁可丢样本也不能让识别路径等。
DEFAULT_QUEUE_MAX = 256
# 目录总容量上限；超过即停止写入（不是删旧的——旧的是语料）。
DEFAULT_MAX_BYTES = 8 * 1024 * 1024 * 1024  # 8 GiB


def _day() -> str:
    return datetime.now(timezone.utc).astimezone().strftime("%Y-%m-%d")


def sample_is_complete(record: dict, root) -> bool:
    """评测侧的入选判据：JSONL 行有了，且引用的 wav 都能完整读出、帧数对得上。

    只检查「文件存在」不够——写到一半崩溃同样会留下存在但截断的文件。
    """
    root = Path(root)
    for path_key, frame_key in (("wav_ctx", "ctx_frames"), ("wav_plain", "plain_frames")):
        rel = record.get(path_key)
        want = record.get(frame_key)
        if rel is None and want is None:
            continue  # 这条记录本就没有音频（例如 stage=no_prev）
        if rel is None or want is None:
            return False
        f = root / rel
        if not f.exists():
            return False
        try:
            info = sf.info(str(f))
        except Exception:
            return False
        if info.frames != want:
            return False
    return True


class CtxObserver:
    """把一条观测记录（JSON + 最多两段音频）异步、原子地落盘。"""

    def __init__(
        self,
        root,
        sr: int = 16000,
        queue_max: int = DEFAULT_QUEUE_MAX,
        max_bytes: int = DEFAULT_MAX_BYTES,
        _start_worker: bool = True,
    ):
        self.root = Path(root)
        self.sr = sr
        self.max_bytes = max_bytes
        self._q: queue.Queue = queue.Queue(maxsize=queue_max)
        self._stats = {"written": 0, "dropped": 0, "write_errors": 0, "over_budget": 0}
        self._lock = threading.Lock()
        self._worker = None
        if _start_worker:
            self._worker = threading.Thread(target=self._run, name="ctx-observe", daemon=True)
            self._worker.start()

    # ---- 调用方接口 ----------------------------------------------------

    def submit(self, record: dict, ctx_audio, plain_audio) -> bool:
        """入队一条记录。**永不阻塞、永不抛**；队列满返回 False 并计数。"""
        try:
            self._q.put_nowait((record, ctx_audio, plain_audio))
            return True
        except queue.Full:
            with self._lock:
                self._stats["dropped"] += 1
            return False
        except Exception:  # noqa: BLE001 — 观测绝不能影响识别
            with self._lock:
                self._stats["dropped"] += 1
            return False

    def stats(self) -> dict:
        with self._lock:
            return dict(self._stats)

    def close(self, timeout: float = 10.0):
        """把队列里剩下的写完（测试与优雅退出用）。"""
        if self._worker is None:
            self._drain_inline()
            return
        self._q.put(None)
        self._worker.join(timeout)

    # ---- 后台写 --------------------------------------------------------

    def _run(self):
        while True:
            item = self._q.get()
            if item is None:
                return
            self._write_one(*item)

    def _drain_inline(self):
        while True:
            try:
                item = self._q.get_nowait()
            except queue.Empty:
                return
            if item is not None:
                self._write_one(*item)

    def _dir_bytes(self) -> int:
        return sum(f.stat().st_size for f in self.root.rglob("*") if f.is_file())

    def _write_one(self, record: dict, ctx_audio, plain_audio):
        try:
            if self.root.exists() and self._dir_bytes() >= self.max_bytes:
                with self._lock:
                    self._stats["over_budget"] += 1
                return
        except OSError:
            pass

        day = self.root / _day()
        key = str(record.get("key") or "unkeyed")
        written: list[Path] = []
        try:
            day.mkdir(parents=True, exist_ok=True)
            try:
                os.chmod(self.root, 0o700)  # 用户语音，别让别的账号读到
            except OSError:
                pass

            for field, frames_field, audio in (
                ("wav_ctx", "ctx_frames", ctx_audio),
                ("wav_plain", "plain_frames", plain_audio),
            ):
                if audio is None:
                    record[field] = None
                    record[frames_field] = None
                    continue
                arr = np.asarray(audio, dtype=np.float32)
                name = f"{key}.{field}.wav"
                final = day / name
                tmp = day / (name + ".tmp")
                # float32 存盘：模型收到的就是 float32，存 PCM16 会引入量化差异，
                # 那就不再是「生产当时真正的输入」。
                # 显式给 format：临时文件是 .tmp 后缀，soundfile 推断不出格式。
                sf.write(str(tmp), arr, self.sr, format="WAV", subtype="FLOAT")
                self._fsync(tmp)
                tmp.replace(final)
                written.append(final)
                record[field] = str(final.relative_to(self.root)).replace("\\", "/")
                record[frames_field] = int(arr.shape[0])

            record.setdefault("ts", datetime.now(timezone.utc).astimezone().isoformat())
            line = json.dumps(record, ensure_ascii=False)
            jsonl = day / "ctx.jsonl"
            with open(jsonl, "a", encoding="utf-8") as fh:
                fh.write(line + "\n")
                fh.flush()
                os.fsync(fh.fileno())
            with self._lock:
                self._stats["written"] += 1
        except Exception as e:  # noqa: BLE001
            # 中途失败：JSONL 行没写成，所以这条样本「不存在」。把已落地的 wav
            # 和临时文件清掉，免得留下无主文件被误当成完整样本。
            for f in written:
                try:
                    f.unlink()
                except OSError:
                    pass
            for f in day.glob(f"{key}.*.tmp"):
                try:
                    f.unlink()
                except OSError:
                    pass
            with self._lock:
                self._stats["write_errors"] += 1
            print(f"[asr][ctx-obs] write failed key={key}: {e}", flush=True)

    @staticmethod
    def _fsync(path: Path):
        """尽力刷盘。Windows 上 fsync 需要可写句柄，只读句柄会 EBADF。

        刷不动不算失败：JSONL 行才是提交标记，而评测侧还会实际读一遍 wav
        校验帧数（`sample_is_complete`），没刷干净的残缺文件在那一步会被排除。
        """
        try:
            fd = os.open(str(path), os.O_RDWR)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        except OSError:
            pass
