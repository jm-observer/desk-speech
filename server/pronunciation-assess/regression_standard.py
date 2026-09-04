"""标准音频回归:用 TTS 标准发音做发音评测的「穷人版标定基准」。

背景:2026-07-02 实测发现母语 TTS 的 `My shoes hurt` 只考 0.63(线 0.6,余量 0.03)、
句尾不除阻 /t/ 被判 bad——评分体系冤枉标准发音(toolkit docs/english-shadow-todo.md
「标定实测发现」)。本脚本把该实验固化:每次改判分规则/标定后跑一遍,防止修了短语坏了长句。

断言(通过判定与 toolkit 聚合同源:句分 >= 0.6 且 bad <= max(1, 音素数//10)):
- **母语声(en_m_3752 / en_f_5895)全部短语必须通过,且句分余量 >= MARGIN(0.10)**;
- 中文口音声(edge_yunxi)仅打印参考,不断言(它读英文本来可能不准);
- 故意错读探针:`I sink so` 按 `I think so` 评,**TH 必须仍判 bad**(豁免规则不得漏判真错读)。

运行(GB10 主机,依赖 :8095 TTS + :8098 assess 均在跑):
    python3 regression_standard.py
退出码:0 全过;1 有断言失败。仅标准库(urllib),无第三方依赖。
"""
from __future__ import annotations

import io
import json
import sys
import urllib.request
import uuid

TTS_URL = "http://127.0.0.1:8095/tts"
ASSESS_URL = "http://127.0.0.1:8098/assess"

# 通过判定,与 toolkit `shadow::max_bad_phones` + DEFAULT_THRESHOLD 同源。
THRESHOLD = 0.6
MARGIN = 0.10  # 母语声要求的句分余量:标准发音不该擦线过

NATIVE_VOICES = ["en_m_3752", "en_f_5895"]  # LibriTTS-R 母语男女声 → 必须通过
ACCENT_VOICES = ["edge_yunxi"]              # 中文口音 → 仅参考

# 覆盖:短语(实测踩坑)/ 塞音结尾 / 摩擦音 / 卷舌元音 / 中长句。
PHRASES = [
    "My shoes hurt",              # 实测案例:句尾 T + ER
    "What is this",
    "I think so",                 # TH
    "Take a look at it",          # 多个词尾塞音 T/K
    "The weather is really good today",
    "Could you please turn down the music",
    # /ɑ/(AA)守卫:AA 在模型 vocab 里是 id 0,与 merge_tokens 的 blank 默认值撞号——曾因此把
    # 所有 AA 的 span 当 blank 剔掉,从句中第一个 AA 起全体错位(2026-08-20 修)。原回归短语
    # 里恰好一个 AA 都没有,bug 才逃逸这么久。这两句务必保留,且必须含 AA。
    "These products are made of milk",   # AA×2(products/are)+ dark L + 词尾 K
    "The car is not far from the park",  # AA 密集(car/not/far/park)
]

# 故意错读探针:(合成文本, 评测参考, 必须仍判 bad 的音素)。
# 全部换在**实义词的重读音节**上(豁免规则只覆盖词尾辅音/弱读元音,这些必须仍能抓)。
ERROR_PROBES = [
    ("I sink so", "I think so", "TH"),          # th→s(辅音换位置,中式经典)
    ("My shows hurt", "My shoes hurt", "UW"),   # 重读元音 u→oʊ
    ("It is a sheep", "It is a ship", "IH"),    # 重读元音 ɪ→i(ship/sheep 最小对)
    ("I have a pan", "I have a pen", "EH"),     # 重读元音 ɛ→æ
    ("That is a big lock", "That is a big rock", "R"),  # r→l(中式经典)
]

# 多处错读探针:(合成文本, 评测参考)→ 整句必须**不通过**(单音素错读按放宽规则可容 1 个,
# 但错两处及以上不能混过)。
MULTI_ERROR_PROBES = [
    ("My shows heart", "My shoes hurt"),   # UW→OW + ER→AA 两处
    ("I sink you are light", "I think you are right"),  # TH→S + R→L 两处
]


def tts(text: str, voice_id: str) -> bytes:
    body = json.dumps({"text": text, "voice_id": voice_id}).encode()
    req = urllib.request.Request(
        TTS_URL, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def assess(audio: bytes, ref_text: str) -> dict:
    """multipart 上传(仅标准库,手拼 boundary)。"""
    boundary = uuid.uuid4().hex
    buf = io.BytesIO()

    def field(name: str, value: str) -> None:
        buf.write(f"--{boundary}\r\nContent-Disposition: form-data; "
                  f"name=\"{name}\"\r\n\r\n{value}\r\n".encode())

    buf.write(f"--{boundary}\r\nContent-Disposition: form-data; "
              f"name=\"audio\"; filename=\"a.wav\"\r\n"
              f"Content-Type: audio/wav\r\n\r\n".encode())
    buf.write(audio)
    buf.write(b"\r\n")
    field("ref_text", ref_text)
    buf.write(f"--{boundary}--\r\n".encode())
    req = urllib.request.Request(
        ASSESS_URL, data=buf.getvalue(),
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.loads(r.read())


def phone_total(resp: dict) -> int:
    return sum(len(w.get("phones") or []) for w in resp.get("words", []))


def passed(resp: dict) -> bool:
    # 与 toolkit `shadow::max_bad_phones` 同源:每 10 音素容 1 个 bad,短句(<10)零容忍。
    quota = phone_total(resp) // 10
    return (resp["sentence_score"] >= THRESHOLD
            and resp.get("bad_phone_count", 0) <= quota)


def brief(resp: dict) -> str:
    bads = [f"{w['ref']}/{p['ph']}={p['score']:.2f}"
            for w in resp.get("words", [])
            for p in (w.get("phones") or [])
            if p.get("pron_status") == "bad"]
    return f"句分={resp['sentence_score']:.3f} bad={resp.get('bad_phone_count', 0)}" + (
        f" [{', '.join(bads)}]" if bads else "")


def main() -> int:
    failures: list[str] = []

    for voice in NATIVE_VOICES + ACCENT_VOICES:
        native = voice in NATIVE_VOICES
        print(f"\n== {voice}{'(母语,断言)' if native else '(口音,仅参考)'} ==")
        for text in PHRASES:
            resp = assess(tts(text, voice), text)
            ok = passed(resp) and resp["sentence_score"] - THRESHOLD >= MARGIN
            if native and not ok:
                # CosyVoice zero-shot 合成偶发坏音频(实测同句同声可从 0.9 掉到 0.05)
                # → 重合成一次再判,连败才算真失败。
                resp = assess(tts(text, voice), text)
                ok = passed(resp) and resp["sentence_score"] - THRESHOLD >= MARGIN
            print(f"  {'PASS' if ok else 'FAIL'}  {text!r}  {brief(resp)}")
            if native and not passed(resp):
                failures.append(f"{voice} {text!r} 未通过: {brief(resp)}")
            elif native and resp["sentence_score"] - THRESHOLD < MARGIN:
                failures.append(
                    f"{voice} {text!r} 余量不足({resp['sentence_score'] - THRESHOLD:.3f} < {MARGIN}): {brief(resp)}")

    print("\n== 故意错读探针(豁免规则不得漏判真错读) ==")
    for synth_text, ref_text, must_bad in ERROR_PROBES:
        resp = assess(tts(synth_text, NATIVE_VOICES[0]), ref_text)
        bad_phs = {p["ph"] for w in resp.get("words", [])
                   for p in (w.get("phones") or [])
                   if p.get("pron_status") == "bad"}
        hit = must_bad in bad_phs and not passed(resp)
        print(f"  {'PASS' if hit else 'FAIL'}  合成 {synth_text!r} 按 {ref_text!r} 评 "
              f"→ {brief(resp)} 整句{'通过' if passed(resp) else '不通过'}(要求 bad 含 {must_bad} 且整句不通过)")
        if must_bad not in bad_phs:
            failures.append(f"错读探针 {synth_text!r}: {must_bad} 未判 bad(豁免过宽,漏判)")
        elif passed(resp):
            failures.append(f"错读探针 {synth_text!r}: 判出了 bad 但整句仍通过(配额过宽)")

    print("\n== 多处错读探针(整句必须不通过) ==")
    for synth_text, ref_text in MULTI_ERROR_PROBES:
        resp = assess(tts(synth_text, NATIVE_VOICES[0]), ref_text)
        ok = not passed(resp)
        print(f"  {'PASS' if ok else 'FAIL'}  合成 {synth_text!r} 按 {ref_text!r} 评 → {brief(resp)}")
        if not ok:
            failures.append(f"多错读探针 {synth_text!r} 整句竟然通过了(过宽): {brief(resp)}")

    print()
    if failures:
        print("回归失败:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("回归全部通过。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
