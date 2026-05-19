# GB10 shim, auto-imported by Python at startup (must be on sys.path).
# The cu130 torchaudio paired with the GB10 torch dropped the soundfile
# backend and forces torchcodec, which has no arm64 wheel. GPT-SoVITS calls
# torchaudio.load/save internally, so route them through soundfile globally.
try:
    import soundfile as _sf
    import torch as _t
    import torchaudio as _ta

    def _load(fp, *a, **k):
        d, s = _sf.read(fp, dtype="float32", always_2d=True)  # (T, C)
        return _t.from_numpy(d.T).contiguous(), s  # (C, T), sr

    def _save(fp, src, sr, *a, **k):
        x = src.detach().cpu().numpy() if hasattr(src, "detach") else src
        _sf.write(fp, x.T if getattr(x, "ndim", 1) == 2 else x, sr)

    _ta.load = _load
    _ta.save = _save
except Exception as e:  # never block startup on the shim
    import sys

    print("sitecustomize torchaudio shim failed:", e, file=sys.stderr)
