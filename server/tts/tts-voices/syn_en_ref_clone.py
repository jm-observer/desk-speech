"""English zero-shot clone bake-off: every English ref voice x every English
test sentence, via CosyVoice2 /tts/zero_shot.

Companion to gen_en_refs.py — runs the full matrix over the LibriTTS-R English
refs in ./refs/en/ and the English sentence set in ./en_sentences.json. Output
goes to ./outputs/en_ref_clone/<voice_id>/<sent_id>.wav (outputs/ is gitignored,
see README.md for layout).

Inputs:
- ./refs/en/en_voices.json   (refs + prompt_text_override)
- ./en_sentences.json        (id/label/text)

Usage:
  python syn_en_ref_clone.py                 # default 2 refs (1F/1M) x all sentences
  python syn_en_ref_clone.py --all-voices    # all 6 refs
  TTS_URL=http://192.168.0.68:8095 python syn_en_ref_clone.py

CosyVoice2 is GPU-bound and not concurrency-safe -> calls run serially.
"""
import argparse
import json
import os
import sys
from pathlib import Path

import requests

HERE = Path(__file__).parent
REFS_DIR = HERE / "refs" / "en"
OUT_DIR = HERE / "outputs" / "en_ref_clone"
TTS_URL = os.environ.get("TTS_URL", "http://192.168.0.68:8095")
DEFAULT_VOICES = {"en_f_84", "en_m_2803"}  # 1 female + 1 male, both 5-10s


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--all-voices", action="store_true",
                    help="synthesize with all 6 refs (default: 1F + 1M)")
    args = ap.parse_args()

    voices = json.loads((REFS_DIR / "en_voices.json").read_text("utf-8"))["voices"]
    if not args.all_voices:
        voices = [v for v in voices if v["id"] in DEFAULT_VOICES]
    sentences = json.loads((HERE / "en_sentences.json").read_text("utf-8"))

    h = requests.get(f"{TTS_URL}/health", timeout=10).json()
    print(f"[health] {h}")
    if not h.get("ok"):
        sys.exit("TTS service not healthy")

    for v in voices:
        ref = REFS_DIR / v["file"]
        vdir = OUT_DIR / v["id"]
        vdir.mkdir(parents=True, exist_ok=True)
        for s in sentences:
            with ref.open("rb") as fh:
                r = requests.post(
                    f"{TTS_URL}/tts/zero_shot",
                    data={"tts_text": s["text"],
                          "prompt_text": v["prompt_text_override"]},
                    files={"prompt_wav": (v["file"], fh, "audio/wav")},
                    timeout=240,
                )
            out = vdir / f"{s['id']}.wav"
            if r.status_code == 200 and r.content[:4] == b"RIFF":
                out.write_bytes(r.content)
                print(f"  ok  {v['id']}/{out.name}  {len(r.content)//1024}KB")
            else:
                print(f"  FAIL {v['id']}/{out.name} "
                      f"http={r.status_code} {r.text[:120]}")
    print(f"done -> {OUT_DIR}")


if __name__ == "__main__":
    main()
