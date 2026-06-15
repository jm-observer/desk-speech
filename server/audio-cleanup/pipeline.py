"""音频清洗管线（Plan 2）。

设计契约见 docs/2026-06-14-audio-cleanup/audio-cleanup-plan-2.md。要点：

  顺序固定：decode → separate(可选) → denoise(可选) → vad → loudness → encode
  采样率铁律：DeepFilterNet 只支持 48kHz —— DF stage 内部固定 48k mono 处理；
              请求的 `sr` 只在最后 encode 一步生效，绝不在 DF 前按 sr 降采样。

所有重 ML 依赖（torch/demucs/deepfilternet/onnxruntime/pyloudnorm）都在函数内部惰性 import，
使本模块在无这些依赖的开发机上也能被 import / py_compile（单测纯逻辑、stage 逻辑在 GB10 上验）。

本模块同时是「可终止子进程」的入口（见底部 __main__）：app.py 用 create_subprocess_exec
拉起它，超时即 kill —— 模型在子进程内加载、随子进程退出释放，空闲 GPU 零占用。
"""
from __future__ import annotations

import dataclasses
import json
import os
import subprocess
import sys

# DeepFilterNet 固定工作采样率。不可改 —— deep-filter 当前仅支持 48k。
DF_SR = 48000
# silero VAD 工作采样率（仅用于检测语音区间；增益施加回 48k 时间轴）。
VAD_SR = 16000

VALID_PAUSE = ("drop", "duck", "off")
VALID_LEVEL = ("gentle", "balanced", "aggressive")
VALID_FORMAT = ("wav", "mp3", "flac")

# Demucs device 由 Plan 1 结论拍板；默认方案 A（cpu，GPU 零占用）。
DEMUCS_DEVICE = os.environ.get("CLEAN_DEMUCS_DEVICE", "cpu")
# duck 模式下非语音段的增益（线性）。-20dB ≈ 0.1。
DUCK_GAIN = float(os.environ.get("CLEAN_DUCK_GAIN", "0.1"))


@dataclasses.dataclass
class CleanOpts:
    separate: bool = False
    denoise: bool = True
    pause: str = "duck"          # drop | duck | off
    level: str = "balanced"      # gentle | balanced | aggressive
    loudness: float | None = -16.0  # None = off
    sr: int = 48000
    fmt: str = "wav"             # wav | mp3 | flac

    def validate(self) -> None:
        if self.pause not in VALID_PAUSE:
            raise ValueError(f"bad pause: {self.pause}")
        if self.level not in VALID_LEVEL:
            raise ValueError(f"bad level: {self.level}")
        if self.fmt not in VALID_FORMAT:
            raise ValueError(f"bad format: {self.fmt}")
        if self.sr not in (16000, 24000, 48000):
            raise ValueError(f"bad sr: {self.sr}")

    def to_dict(self) -> dict:
        return dataclasses.asdict(self)

    @staticmethod
    def from_dict(d: dict) -> "CleanOpts":
        return CleanOpts(**d)


# ----------------------------------------------------------------------------
# 时长探测（parent 在 spawn 前调用，便于 422 早拒）—— 只依赖 ffprobe，无 ML 依赖。
# ----------------------------------------------------------------------------
def probe_duration_sec(path: str) -> float:
    """用 ffprobe 取音频时长（秒）。失败抛 RuntimeError。"""
    out = subprocess.run(
        [
            "ffprobe", "-v", "error", "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1", path,
        ],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        raise RuntimeError(f"ffprobe failed: {out.stderr.strip()[:200]}")
    try:
        return float(out.stdout.strip())
    except ValueError as exc:
        raise RuntimeError(f"ffprobe bad duration: {out.stdout.strip()[:80]}") from exc


# ----------------------------------------------------------------------------
# Stage 实现（均惰性 import 重依赖）。每个 stage 输入/输出 (mono float32 ndarray, sr)。
# ----------------------------------------------------------------------------
def stage_decode(input_path: str, want_stereo: bool):
    """ffmpeg 解码任意容器 → 48k float32。separate 需要立体声给 Demucs。"""
    import numpy as np

    ch = 2 if want_stereo else 1
    proc = subprocess.run(
        [
            "ffmpeg", "-nostdin", "-i", input_path,
            "-f", "f32le", "-acodec", "pcm_f32le",
            "-ac", str(ch), "-ar", str(DF_SR), "-",
        ],
        capture_output=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"decode failed: {proc.stderr.decode('utf-8', 'ignore')[:200]}")
    # frombuffer 返回只读数组；下游 torch.from_numpy 对只读 buffer 会告警，这里 copy 成可写。
    audio = np.frombuffer(proc.stdout, dtype=np.float32).copy()
    if ch == 2:
        audio = audio.reshape(-1, 2).T  # (2, n)
    return audio, DF_SR


def stage_separate(audio, sr: int):
    """Demucs htdemucs 取 vocals 轨，下混为 mono。输入 (2,n)@48k，输出 (mono, 48k)。"""
    import numpy as np
    import torch
    from demucs.apply import apply_model
    from demucs.audio import convert_audio
    from demucs.pretrained import get_model

    model = get_model("htdemucs")
    model.to(DEMUCS_DEVICE).eval()

    wav = torch.from_numpy(np.ascontiguousarray(audio)).float()
    if wav.dim() == 1:
        wav = wav.unsqueeze(0).repeat(2, 1)
    wav = convert_audio(wav, sr, model.samplerate, model.audio_channels)
    ref = wav.mean(0)
    wav = (wav - ref.mean()) / (ref.std() + 1e-8)
    with torch.no_grad():
        sources = apply_model(
            model, wav[None].to(DEMUCS_DEVICE), device=DEMUCS_DEVICE, progress=False
        )[0]
    vocals = sources[list(model.sources).index("vocals")]   # (channels, n) @ model.samplerate
    vocals = vocals * (ref.std() + 1e-8) + ref.mean()
    # 回 48k mono
    vocals = convert_audio(vocals.cpu(), model.samplerate, DF_SR, 1)[0]
    return vocals.numpy().astype(np.float32), DF_SR


def _ensure_df_torchaudio_shim():
    """DeepFilterNet 0.5.6 的 df.io 仍 `from torchaudio.backend.common import AudioMetaData`，
    但 GB10 的 dev torchaudio 2.11 已删 `torchaudio.backend`。补一个兼容 shim 模块，把
    AudioMetaData 指到现 torchaudio 里的实现（找不到则给最小 stub，仅满足 import；我们只把
    张量喂给 enhance()，不走 df.io 的 load/save 音频路径）。"""
    import sys
    import types

    import torchaudio

    try:
        from torchaudio.backend.common import AudioMetaData  # noqa: F401

        return  # 已可用，无需 shim
    except Exception:  # noqa: BLE001
        pass

    amd = getattr(torchaudio, "AudioMetaData", None)
    if amd is None:
        try:
            from torchaudio._backend.common import AudioMetaData as amd  # type: ignore
        except Exception:  # noqa: BLE001

            class amd:  # type: ignore  # 最小 stub，仅满足 import
                def __init__(self, *args, **kwargs):
                    pass

    backend = sys.modules.get("torchaudio.backend") or types.ModuleType("torchaudio.backend")
    common = types.ModuleType("torchaudio.backend.common")
    common.AudioMetaData = amd
    backend.common = common
    sys.modules["torchaudio.backend"] = backend
    sys.modules["torchaudio.backend.common"] = common
    torchaudio.backend = backend


def stage_denoise(audio, sr: int, level: str):
    """DeepFilterNet 降噪+去混响。输入必须 48k mono。"""
    import numpy as np
    import torch

    _ensure_df_torchaudio_shim()
    from df.enhance import enhance, init_df

    assert sr == DF_SR, f"DF requires {DF_SR}, got {sr}"
    model, df_state, _ = init_df()
    tensor = torch.from_numpy(np.ascontiguousarray(audio)).float().unsqueeze(0)
    # level → 衰减上限（dB）：gentle 留底噪，aggressive 不限。
    atten = {"gentle": 12.0, "balanced": 24.0, "aggressive": 100.0}[level]
    out = enhance(model, df_state, tensor, atten_lim_db=atten)
    return out.squeeze(0).numpy().astype(np.float32), DF_SR


def stage_vad(audio, sr: int, pause: str):
    """silero VAD：drop 删非语音段 / duck 压低 / off 不动。输入 48k mono。

    用官方 `silero-vad` 包（模型随 wheel 内置、无需联网、自动适配模型版本），
    避免手搓 onnx state（v4 用 h/c、v5 用单 state，签名不同易踩坑）。
    """
    import numpy as np
    import torch
    from silero_vad import get_speech_timestamps, load_silero_vad

    if pause == "off":
        return audio, sr

    # 重采样到 16k 仅用于检测区间；增益施加回 48k 时间轴。
    ratio = VAD_SR / sr
    idx = (np.arange(int(len(audio) * ratio)) / ratio).astype(np.int64)
    idx = np.clip(idx, 0, max(len(audio) - 1, 0))
    audio16 = audio[idx]

    model = load_silero_vad()
    ts = get_speech_timestamps(
        torch.from_numpy(np.ascontiguousarray(audio16)).float(),
        model, sampling_rate=VAD_SR,
    )  # [{'start': samples16, 'end': samples16}]

    mask = np.zeros(len(audio), dtype=np.float32)
    for seg in ts:
        s = int(seg["start"] / ratio)
        e = int(seg["end"] / ratio)
        mask[s:min(e, len(audio))] = 1.0

    if pause == "drop":
        dropped = audio[mask > 0]
        return dropped, sr
    # duck：非语音段乘 DUCK_GAIN（保留节奏）
    gain = np.where(mask > 0, 1.0, DUCK_GAIN).astype(np.float32)
    return (audio * gain).astype(np.float32), sr


def stage_loudness(audio, sr: int, target_lufs: float):
    """pyloudnorm EBU R128 归一化 + 削峰防 clip。返回 (audio, in_lufs, out_lufs)。"""
    import numpy as np
    import pyloudnorm as pyln

    meter = pyln.Meter(sr)
    in_lufs = float(meter.integrated_loudness(audio))
    if not np.isfinite(in_lufs):
        return audio, in_lufs, in_lufs
    normalized = pyln.normalize.loudness(audio, in_lufs, target_lufs)
    peak = float(np.max(np.abs(normalized))) if len(normalized) else 0.0
    if peak > 0.99:
        normalized = normalized * (0.99 / peak)
    out_lufs = float(meter.integrated_loudness(normalized))
    return normalized.astype(np.float32), in_lufs, out_lufs


def stage_encode(audio, sr: int, out_path: str, out_sr: int, fmt: str):
    """末端重采样到 out_sr 并按 fmt 编码（ffmpeg）。这是 sr 唯一生效处。"""
    import numpy as np

    raw = np.ascontiguousarray(audio.astype(np.float32)).tobytes()
    codec = {"wav": "pcm_s16le", "mp3": "libmp3lame", "flac": "flac"}[fmt]
    proc = subprocess.run(
        [
            "ffmpeg", "-nostdin", "-y",
            "-f", "f32le", "-ar", str(sr), "-ac", "1", "-i", "-",
            "-ar", str(out_sr), "-ac", "1", "-acodec", codec, "-f", fmt, out_path,
        ],
        input=raw, capture_output=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"encode failed: {proc.stderr.decode('utf-8', 'ignore')[:200]}")


# ----------------------------------------------------------------------------
# 完整管线（子进程内执行）。
# ----------------------------------------------------------------------------
def run_pipeline(input_path: str, output_path: str, opts: CleanOpts) -> dict:
    """跑完整清洗链，写 output_path，返回元数据 dict（stages/in_lufs/out_lufs）。"""
    opts.validate()
    stages = ["decode"]
    audio, sr = stage_decode(input_path, want_stereo=opts.separate)

    if opts.separate:
        audio, sr = stage_separate(audio, sr)
        stages.append("separate")
    elif audio.ndim == 2:
        audio = audio.mean(axis=0)  # 未分离也要下混成 mono

    if opts.denoise:
        audio, sr = stage_denoise(audio, sr, opts.level)
        stages.append("denoise")

    if opts.pause != "off":
        audio, sr = stage_vad(audio, sr, opts.pause)
        stages.append(f"vad-{opts.pause}")

    in_lufs = out_lufs = float("nan")
    if opts.loudness is not None:
        audio, in_lufs, out_lufs = stage_loudness(audio, sr, opts.loudness)
        stages.append("loudness")

    stage_encode(audio, sr, output_path, opts.sr, opts.fmt)
    stages.append("encode")
    return {"stages": stages, "in_lufs": in_lufs, "out_lufs": out_lufs}


def _main() -> int:
    """子进程入口：argv = input_path output_path opts_json meta_out_path。"""
    input_path, output_path, opts_json, meta_path = sys.argv[1:5]
    opts = CleanOpts.from_dict(json.loads(opts_json))
    meta = run_pipeline(input_path, output_path, opts)
    with open(meta_path, "w", encoding="utf-8") as f:
        json.dump(meta, f)
    return 0


if __name__ == "__main__":
    sys.exit(_main())
