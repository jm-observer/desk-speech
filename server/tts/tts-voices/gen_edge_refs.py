"""Generate clean Chinese voice ref clips with Edge TTS for use as
CosyVoice2/GPT-SoVITS zero-shot prompts.

Notes:
- Edge TTS audio is studio-clean (~70dB SNR), 24kHz mono — good prompt material.
- License caveat: Microsoft Edge TTS output is intended for personal/dev use.
  For production voice library, replace with AISHELL-3 / CC-BY clips.
- One fixed sentence so prompt_text is known and identical across voices.
"""
import asyncio
import edge_tts
import json
from pathlib import Path

OUT_DIR = Path(__file__).parent / "refs" / "zh"
OUT_DIR.mkdir(parents=True, exist_ok=True)

# Fixed prompt sentence — content-rich, no awkward stops, ~5-7s when spoken
PROMPT_TEXT = "今天的会议讨论了下个季度的重点工作和团队分工安排。"

# Curated voice selection: cover gender + tone variety
VOICES = [
    # id,            edge_tts voice name,       gender, tone
    ("xiaoxiao", "zh-CN-XiaoxiaoNeural",  "F", "温暖"),   # warm, default female
    ("xiaoyi",   "zh-CN-XiaoyiNeural",    "F", "活泼"),   # young, lively
    ("yunxi",    "zh-CN-YunxiNeural",     "M", "活泼"),   # cheerful young man
    ("yunjian",  "zh-CN-YunjianNeural",   "M", "中性"),   # neutral male, sports anchor
    ("yunyang",  "zh-CN-YunyangNeural",   "M", "严肃"),   # news anchor, serious
]


async def gen_one(vid, edge_name, gender, tone):
    out = OUT_DIR / f"edge_{vid}.wav"
    if out.exists():
        print(f"  skip {vid} (exists)")
        return
    # edge-tts outputs mp3 by default; ask for raw 24khz 16-bit pcm wav
    comm = edge_tts.Communicate(PROMPT_TEXT, edge_name)
    # Write to .mp3 first, then convert via pydub/ffmpeg? Simpler: save_mp3 then
    # rely on user's environment. But we want wav for cosyvoice prompt.
    mp3_path = out.with_suffix(".mp3")
    await comm.save(str(mp3_path))
    # Convert mp3 -> 24kHz mono wav via ffmpeg (assumed installed)
    import subprocess
    r = subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-i", str(mp3_path),
         "-ac", "1", "-ar", "24000", "-sample_fmt", "s16", str(out)],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        print(f"  FAIL {vid}: {r.stderr}")
        return
    mp3_path.unlink()
    print(f"  ok  {vid:10s} ({gender}/{tone}) -> {out.name}")


async def main():
    print(f"prompt_text: {PROMPT_TEXT}")
    print(f"output dir : {OUT_DIR}")
    manifest = {
        "prompt_text": PROMPT_TEXT,
        "voices": [],
    }
    for vid, edge_name, gender, tone in VOICES:
        await gen_one(vid, edge_name, gender, tone)
        manifest["voices"].append({
            "id": f"edge_{vid}",
            "file": f"edge_{vid}.wav",
            "source": "edge-tts",
            "source_voice": edge_name,
            "gender": gender,
            "tone": tone,
            "license": "Microsoft Edge TTS (dev/personal use)",
        })
    # Append the official cosy ref to manifest
    manifest["voices"].append({
        "id": "cosy_zero_shot",
        "file": "cosy_zero_shot.wav",
        "source": "cosyvoice2-official",
        "source_voice": "asset/zero_shot_prompt.wav",
        "gender": "F",
        "tone": "活泼",
        "license": "Apache-2.0",
        "prompt_text_override": "希望你以后能够做的比我还好呦。",
    })
    (OUT_DIR / "voices.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"manifest -> {OUT_DIR / 'voices.json'}")


if __name__ == "__main__":
    asyncio.run(main())
