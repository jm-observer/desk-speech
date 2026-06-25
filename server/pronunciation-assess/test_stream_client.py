"""流式发音评测 WS 测试客户端(Phase 2a 验证 /assess/stream)。

读 wav → 解码 16k 单声道 → 按 chunk 实时节奏推送 → 打印 ready/partial/final 及到达相对时刻。
用法(容器内):python test_stream_client.py <audio> "<ref_text>" [--chunk-ms 320] [--realtime]
"""
import argparse
import asyncio
import json
import sys
import time

import numpy as np
from aiohttp import ClientSession

import gop


async def run(audio, ref_text, chunk_ms, realtime, port):
    wav = gop._load_audio_16k(audio).numpy()
    pcm = (np.clip(wav, -1, 1) * 32767).astype(np.int16)
    chunk = int(chunk_ms * gop.TARGET_SR / 1000)
    t0 = time.perf_counter()

    def rel():
        return f"{(time.perf_counter()-t0)*1000:6.0f}ms"

    async with ClientSession() as sess:
        async with sess.ws_connect(f"http://127.0.0.1:{port}/assess/stream",
                                   max_msg_size=8 * 1024 * 1024) as ws:
            await ws.send_json({"type": "hello", "ref_text": ref_text, "granularity": "word"})

            async def reader():
                async for msg in ws:
                    d = json.loads(msg.data)
                    if d["type"] == "partial":
                        phs = "".join(
                            f" {p['ph']}={p['score']:.2f}{'!' if p['pron_status']=='bad' else ''}"
                            for p in d.get("phones", []))
                        print(f"[{rel()}] partial  词#{d['word_index']} '{d['ref']}' "
                              f"{d['score']:.2f} {d['pron_status']}{phs}")
                    elif d["type"] == "final":
                        print(f"[{rel()}] FINAL    句分={d['sentence_score']:.2f} "
                              f"bad={d.get('bad_phone_count')} model={d.get('model')}")
                        for w in d["words"]:
                            print(f"            {w['ref']:<10} {w.get('score',0):.2f} {w['pron_status']}")
                        return
                    elif d["type"] == "ready":
                        print(f"[{rel()}] ready")
                    elif d["type"] == "error":
                        print(f"[{rel()}] ERROR {d['message']}"); return

            rd = asyncio.ensure_future(reader())
            for i in range(0, len(pcm), chunk):
                await ws.send_bytes(pcm[i:i + chunk].tobytes())
                if realtime:
                    await asyncio.sleep(chunk_ms / 1000)
            await ws.send_json({"type": "end"})
            await rd


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("audio"); ap.add_argument("ref_text")
    ap.add_argument("--chunk-ms", type=int, default=320)
    ap.add_argument("--realtime", action="store_true", help="按真实节奏 sleep(模拟边说边传)")
    ap.add_argument("--port", type=int, default=8098)
    args = ap.parse_args()
    asyncio.get_event_loop().run_until_complete(
        run(args.audio, args.ref_text, args.chunk_ms, args.realtime, args.port))


if __name__ == "__main__":
    sys.exit(main())
