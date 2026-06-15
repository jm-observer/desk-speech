"""Plan 1 spike: 验证 Demucs(htdemucs) 在 GB10(arm64 + CUDA13 + torch 2.13.dev+cu130) 能否分离人声。

产出（打印到 stdout，回填 spike/README.md 结论表）：
  - torch / torchaudio 版本串（torch 已在 Dockerfile 硬断言；torchaudio 仅打印记录）
  - 对 cuda 与 cpu 两种 device 各跑一次分离：成功/失败、异常类型全文、wall-clock 耗时、
    GPU 显存峰值。

本脚本只验证可行性，不实现任何服务。绝不静默吞 CUDA 异常——异常完整打印正是 spike 的价值。
"""
import os
import sys
import time
import traceback


def _print_versions():
    import torch
    print(f"[ver] torch       = {torch.__version__}")
    try:
        import torchaudio
        # 软校验：仅记录，不断言（dev 版版本串不好匹配）。
        print(f"[ver] torchaudio  = {torchaudio.__version__}")
    except Exception as exc:  # noqa: BLE001 — spike 只为暴露问题
        print(f"[ver] torchaudio  = <import failed: {exc!r}>")
    print(f"[ver] cuda_available = {torch.cuda.is_available()}")


def _run_one(device: str, wav_path: str) -> None:
    """对单个 device 跑一次 htdemucs 分离，打印结论行。"""
    import torch

    print(f"\n===== device={device} =====")
    if device == "cuda" and not torch.cuda.is_available():
        print(f"[{device}] SKIP — cuda not available")
        return

    if device == "cuda":
        torch.cuda.reset_peak_memory_stats()

    t0 = time.monotonic()
    try:
        # 用 demucs 的高层 API：apply_model + 预训练 htdemucs。
        from demucs.pretrained import get_model
        from demucs.apply import apply_model
        from demucs.audio import AudioFile, convert_audio

        model = get_model("htdemucs")
        model.to(device)
        model.eval()

        wav = AudioFile(wav_path).read(
            streams=0, samplerate=model.samplerate, channels=model.audio_channels
        )
        ref = wav.mean(0)
        wav = (wav - ref.mean()) / (ref.std() + 1e-8)
        with torch.no_grad():
            sources = apply_model(
                model, wav[None].to(device), device=device, progress=False
            )[0]
        # htdemucs 的 source 顺序里有 "vocals"
        names = list(model.sources)
        vocals_idx = names.index("vocals") if "vocals" in names else -1
        elapsed = time.monotonic() - t0

        peak = ""
        if device == "cuda":
            peak = f", gpu_peak={torch.cuda.max_memory_allocated()/1e9:.2f}GB"
        print(
            f"[{device}] OK — sources={names}, vocals_idx={vocals_idx}, "
            f"out_shape={tuple(sources.shape)}, elapsed={elapsed:.1f}s{peak}"
        )
    except Exception:  # noqa: BLE001 — spike 必须暴露完整异常
        elapsed = time.monotonic() - t0
        print(f"[{device}] FAIL after {elapsed:.1f}s — full traceback:")
        traceback.print_exc()


def main() -> int:
    wav_path = os.environ.get("SPIKE_WAV", "/spike/test.wav")
    if not os.path.exists(wav_path):
        print(f"ERROR: SPIKE_WAV not found: {wav_path}")
        print("挂载一段测试 wav 到该路径（建议 ~30s，含人声+背景音乐）后重跑。")
        return 2

    _print_versions()
    for device in ("cuda", "cpu"):
        _run_one(device, wav_path)
    print("\n结论：把上面 cuda / cpu 两段填进 spike/README.md 的结论表。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
