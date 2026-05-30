"""Pick clean English voice ref clips from LibriTTS-R for use as
CosyVoice2 zero-shot prompts (English target text).

Why LibriTTS-R (not Edge TTS like gen_edge_refs.py):
- The Chinese voice lib uses Edge TTS refs (dev/personal license only). For
  English we want a redistributable source -> LibriTTS-R is CC BY 4.0.
- LibriTTS-R is restored studio-quality audiobook speech, 24kHz, high SNR,
  with verbatim transcripts -> ideal zero-shot prompt material.

Quality filter (avoids weak prompts like short proper-noun fragments):
- duration 6-11s, >= MIN_WORDS words, ASCII-only, >=60% lowercase tokens
  (i.e. a natural sentence, not a list of capitalized names).

Output: ./refs/en/  with `file == id` naming (en_<f|m>_<reader>.wav), plus
transcripts.json/txt and en_voices.json (voices.json-compatible; each voice
carries prompt_text_override = its verbatim transcript).

Deps: `pip install datasets huggingface_hub` (audio written from raw wav bytes
via Audio(decode=False) -> torchcodec NOT required).

Run:  python gen_en_refs.py

NOTE: the committed refs/en/ set is hand-curated (a couple of readers kept from
an earlier looser pass). A fresh run reproduces an equivalently-clean 3F+3M set
but may pick different readers.
"""
import io
import json
import re
import urllib.request
import wave
from pathlib import Path

from datasets import Audio, load_dataset

OUT_DIR = Path(__file__).parent / "refs" / "en"
SPEAKERS_TXT = OUT_DIR / "_SPEAKERS.TXT"
SPEAKERS_URL = (
    "https://raw.githubusercontent.com/resemble-ai/Resemblyzer/"
    "15d828edebe06bc72b9cabc8ef8ca5ab2cb457ce/"
    "audio_data/librispeech_train-clean-100/SPEAKERS.TXT"
)
NEED = 3                       # distinct speakers per gender
DUR_LO, DUR_HI = 6.0, 11.0
MIN_WORDS = 15
TONE = {
    "84": "叙述-平稳", "5895": "叙述-柔和", "2035": "叙述-清亮",
    "2803": "沉稳-厚重", "3752": "叙述-自然", "2902": "叙述-沉稳",
}


def load_gender_table() -> dict:
    if not SPEAKERS_TXT.exists():
        OUT_DIR.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(SPEAKERS_URL, SPEAKERS_TXT)
    gender = {}
    for line in SPEAKERS_TXT.read_text(encoding="utf-8").splitlines():
        if line.startswith(";"):
            continue
        parts = [p.strip() for p in line.split("|")]
        if len(parts) >= 2 and parts[0].isdigit():
            gender[parts[0]] = parts[1]  # 'M' / 'F'
    return gender


def clip_duration(wav_bytes: bytes):
    try:
        w = wave.open(io.BytesIO(wav_bytes))
        return w.getnframes() / float(w.getframerate())
    except Exception:
        return None


def is_clean_english(text: str) -> bool:
    toks = re.findall(r"[A-Za-z][A-Za-z']*", text)
    if len(toks) < MIN_WORDS:
        return False
    if any(ord(c) > 127 for c in text):
        return False
    return sum(1 for w in toks if w.islower()) / len(toks) >= 0.6


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    gender = load_gender_table()
    ds = load_dataset(
        "mythicinfinity/libritts_r", "clean", split="dev.clean", streaming=True
    ).cast_column("audio", Audio(decode=False))

    picked = {"M": set(), "F": set()}
    recs = []
    for s in ds:
        if len(picked["M"]) >= NEED and len(picked["F"]) >= NEED:
            break
        spk = str(s["speaker_id"])
        g = gender.get(spk)
        if g not in ("M", "F") or spk in picked[g] or len(picked[g]) >= NEED:
            continue
        b = s["audio"]["bytes"]
        d = clip_duration(b)
        if d is None or not (DUR_LO <= d <= DUR_HI):
            continue
        text = (s.get("text_normalized") or s.get("text_original") or "").strip()
        if not is_clean_english(text):
            continue
        fid = f"en_{g.lower()}_{spk}"
        fn = f"{fid}.wav"
        (OUT_DIR / fn).write_bytes(b)
        picked[g].add(spk)
        recs.append({"id": fid, "file": fn, "gender": g, "speaker_id": spk,
                     "dur": round(d, 2), "text": text})
        print(f"  ok  {fn:14s} {d:5.2f}s  {len(text.split()):2d}w  {text[:50]}")

    recs.sort(key=lambda r: (r["gender"], int(r["speaker_id"])))

    # transcripts.json / .txt
    manifest = [{"file": r["file"], "gender": r["gender"],
                 "speaker_id": r["speaker_id"], "dur": r["dur"],
                 "sr": 24000, "text": r["text"]} for r in recs]
    (OUT_DIR / "transcripts.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    with (OUT_DIR / "transcripts.txt").open("w", encoding="utf-8") as f:
        for r in manifest:
            f.write(f"{r['file']}\t{r['gender']}\t{r['dur']}s\t{r['text']}\n")

    # voices.json-compatible manifest (file == id)
    voices = [
        {"id": r["id"], "file": r["file"], "source": "libritts-r",
         "source_voice": f"LibriTTS-R dev-clean reader {r['speaker_id']}",
         "gender": r["gender"], "tone": TONE.get(r["speaker_id"], ""),
         "dur_sec": r["dur"], "license": "CC BY 4.0 (LibriTTS-R / LibriVox)",
         "prompt_text_override": r["text"]}
        for r in recs
    ]
    (OUT_DIR / "en_voices.json").write_text(
        json.dumps(
            {"_comment": "English zero-shot prompt voices from LibriTTS-R "
                         "dev-clean. 24kHz mono 16-bit, single speaker, 6-11s, "
                         "clean natural sentences (>=15 words). file == id.",
             "prompt_text": "", "voices": voices},
            ensure_ascii=False, indent=2),
        encoding="utf-8")
    print(f"manifest -> {OUT_DIR / 'en_voices.json'} ({len(voices)} voices)")


if __name__ == "__main__":
    main()
