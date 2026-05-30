#!/bin/bash
# A/B listening harness for the CosyVoice 2 service.
#   ./compare.sh <ref.wav> "<ref transcript>" ["<sentence to speak>"]
# Produces: zero-shot clone + several instruct (语气/情感) variants of the
# same sentence in the same cloned voice, so you can judge identity + control.
set -euo pipefail

REF_WAV="${1:?usage: compare.sh ref.wav 'ref transcript' ['target sentence']}"
REF_TXT="${2:?ref transcript required}"
SAY="${3:-今天天气不错，我们出去走走吧，顺便聊聊最近的安排。}"
URL="${TTS_URL:-http://localhost:8095}"
OUT="${OUT_DIR:-/io/out}"
mkdir -p "$OUT"

echo "[health] $(curl -s "$URL/health")"

zs() { curl -s -o "$OUT/$1" -F "tts_text=$SAY" -F "prompt_text=$REF_TXT" \
        -F "prompt_wav=@$REF_WAV;type=audio/wav" "$URL/tts/zero_shot" \
      && echo "  -> $OUT/$1"; }
iv() { curl -s -o "$OUT/$1" -F "tts_text=$SAY" -F "instruct=$2" \
        -F "prompt_wav=@$REF_WAV;type=audio/wav" "$URL/tts/instruct" \
      && echo "  -> $OUT/$1 ($2)"; }

echo "[zero-shot clone]";        zs zeroshot.wav
echo "[instruct variants]"
iv happy.wav  "用开心、轻快的语气说"
iv calm.wav   "用平静、温柔的语气慢慢地说"
iv serious.wav "用严肃、低沉的语气说"
iv sichuan.wav "用四川话说"

echo "Done. Pull to listen:  scp -r fengqi@192.168.0.68:~/tts-io/out ./tts-out"
