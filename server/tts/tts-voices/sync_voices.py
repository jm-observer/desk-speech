#!/usr/bin/env python3
"""把 refs/zh 全部 + refs/en 选定音色合并并部署到 GB10 `~/tts-voices/`。

线上 `~/tts-voices/voices.json` 是 **zh + en 的合并产物**。历史上中文那套
直接 `scp refs/zh/*` 部署，但英文 ref 并入后，再直接 `scp refs/zh/voices.json`
会**覆盖掉英文条目**。因此统一用本脚本部署，不要再手动 scp voices.json。

- zh：`refs/zh/voices.json` 全部音色
- en：`refs/en/en_voices.json` 中 `ENABLED_EN` 列出的音色（其余 en ref 仅本地实验）

产物写到 `_deploy/`（gitignored），`--push` 时 scp 到 GB10。`/voices` 热重读，
不用重启容器。

用法：
    python sync_voices.py            # dry-run：在 _deploy/ 生成合并产物，不推送
    python sync_voices.py --push     # 生成 + scp 到 GB10 ~/tts-voices/
"""

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ZH_DIR = HERE / "refs" / "zh"
EN_DIR = HERE / "refs" / "en"
DEPLOY_DIR = HERE / "_deploy"
REMOTE = "fengqi@192.168.0.68:tts-voices/"

# 上线的英文音色 id（refs/en/en_voices.json 里的子集）。其余 en ref 仅本地实验。
# 加 / 减英文音色改这一行即可，然后 `python sync_voices.py --push`。
ENABLED_EN = ["en_m_3752", "en_f_5895"]


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def build_merged() -> dict:
    """合并 zh 全部 + en 选定音色为单个 voices.json 字典。"""
    zh = load_json(ZH_DIR / "voices.json")
    en = load_json(EN_DIR / "en_voices.json")
    en_by_id = {v["id"]: v for v in en["voices"]}

    missing = [vid for vid in ENABLED_EN if vid not in en_by_id]
    if missing:
        sys.exit(f"ENABLED_EN 中存在 en_voices.json 没有的 id: {missing}")

    merged = dict(zh)  # 顶层 prompt_text 沿用 zh 的默认
    merged["voices"] = list(zh["voices"]) + [en_by_id[vid] for vid in ENABLED_EN]
    return merged


def collect_wavs(merged: dict) -> list[Path]:
    """按每个 voice 的 file 名在 zh / en 目录里定位 wav。"""
    files: list[Path] = []
    for voice in merged["voices"]:
        name = voice["file"]
        for directory in (ZH_DIR, EN_DIR):
            candidate = directory / name
            if candidate.exists():
                files.append(candidate)
                break
        else:
            sys.exit(f"找不到 voice '{voice['id']}' 的 wav 文件: {name}")
    return files


def stage(merged: dict, wavs: list[Path]) -> None:
    """在 _deploy/ 生成合并 voices.json + 收集所有 wav。"""
    DEPLOY_DIR.mkdir(exist_ok=True)
    for old in DEPLOY_DIR.iterdir():
        old.unlink()
    (DEPLOY_DIR / "voices.json").write_text(
        json.dumps(merged, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    for wav in wavs:
        shutil.copy2(wav, DEPLOY_DIR / wav.name)


def push(wavs: list[Path]) -> None:
    """scp voices.json + 所有 wav 到 GB10。scp 不删目标多余文件，幂等安全。"""
    targets = [str(DEPLOY_DIR / "voices.json")]
    targets += [str(DEPLOY_DIR / wav.name) for wav in wavs]
    cmd = ["scp", *targets, REMOTE]
    print("$", " ".join(cmd))
    subprocess.run(cmd, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--push",
        action="store_true",
        help="scp 到 GB10（默认只在 _deploy/ 生成，不推送）",
    )
    args = parser.parse_args()

    merged = build_merged()
    wavs = collect_wavs(merged)
    stage(merged, wavs)

    ids = ", ".join(v["id"] for v in merged["voices"])
    print(f"合并 {len(merged['voices'])} 个音色 → {DEPLOY_DIR}")
    print(f"  voices: {ids}")

    if args.push:
        push(wavs)
        print(f"已推送到 {REMOTE}（/voices 热重读，无需重启）")
    else:
        print("dry-run（未推送）。确认无误后加 --push 部署。")


if __name__ == "__main__":
    main()
