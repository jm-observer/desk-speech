#!/usr/bin/env python3
"""Headless GPT-SoVITS v2Pro fine-tune for the TTS bake-off.

Give it the target speaker's raw audio (one file or a folder); it slices,
auto-transcribes (FunASR), extracts features, trains SoVITS (s2) + GPT (s1),
and prints the per-voice weight paths. Optionally hot-loads them into a
running api_v2 so you can immediately A/B.

Run INSIDE the gptsovits container (cwd = repo root), e.g.:
  docker exec gptsovits python3 /io/gptsovits_finetune.py \
      --audio /io/myvoice.wav --exp me --epochs-s2 8 --epochs-s1 15 \
      --serve-url http://127.0.0.1:9880

Mirrors webui.py's open1abc / open1Ba / open1Bb exactly, pinned to v2Pro.
Experiment data + trained weights go under /io/gs_train/<exp>/ (host-persisted
via the /io mount), so the api container can load them over the same volume.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import urllib.request

REPO = "/app/GPT-SoVITS"
PY = sys.executable or "python3"

# v2Pro pretrained set (from config.py / webui.py).
BERT = "GPT_SoVITS/pretrained_models/chinese-roberta-wwm-ext-large"
CNHUBERT = "GPT_SoVITS/pretrained_models/chinese-hubert-base"
SV_PATH = "GPT_SoVITS/pretrained_models/sv/pretrained_eres2netv2w24s4ep4.ckpt"
S2G = "GPT_SoVITS/pretrained_models/v2Pro/s2Gv2Pro.pth"
S2D = "GPT_SoVITS/pretrained_models/v2Pro/s2Dv2Pro.pth"
S1 = "GPT_SoVITS/pretrained_models/s1v3.ckpt"
S2_CFG = "GPT_SoVITS/configs/s2v2Pro.json"
S1_CFG = "GPT_SoVITS/configs/s1longer-v2.yaml"
VERSION = "v2Pro"


def run(cmd, env=None):
    print("+ " + (cmd if isinstance(cmd, str) else " ".join(cmd)), flush=True)
    e = dict(os.environ)
    # prepare_datasets/1-get-text.py does `from text.cleaner ...` with NO
    # path setup, so GPT_SoVITS/ must be on PYTHONPATH; tools.* needs the
    # repo root. s1/s2_train.py likewise import GPT_SoVITS-internal pkgs.
    e["PYTHONPATH"] = "%s:%s/GPT_SoVITS" % (REPO, REPO)
    if env:
        e.update({k: str(v) for k, v in env.items()})
    p = subprocess.run(cmd, shell=isinstance(cmd, str), cwd=REPO, env=e)
    if p.returncode != 0:
        sys.exit("step failed: %s" % cmd)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--audio", required=True, help="target speaker audio: file or folder")
    ap.add_argument("--exp", required=True, help="experiment / voice name")
    ap.add_argument("--lang", default="zh")
    ap.add_argument("--epochs-s2", type=int, default=8)
    ap.add_argument("--epochs-s1", type=int, default=15)
    ap.add_argument("--gpu", default="0")
    ap.add_argument("--batch", type=int, default=4)
    ap.add_argument("--serve-url", default="", help="api_v2 base url to hot-load weights into")
    a = ap.parse_args()

    os.chdir(REPO)
    sys.path.insert(0, REPO)
    from config import is_half  # GB10 -> True (fp16)

    base = "/io/gs_train/%s" % a.exp
    sliced = "%s/sliced" % base
    opt_dir = "%s/exp" % base                 # 1abc feature dir
    wdir = "%s/weights" % base
    # webui.py pre-creates these; s2/s1_train save checkpoints into them.
    for d in (sliced, opt_dir, wdir,
              "%s/logs_s2_%s" % (opt_dir, VERSION),
              "%s/logs_s1_%s" % (opt_dir, VERSION)):
        os.makedirs(d, exist_ok=True)

    # ---- slice (webui slicer defaults) ----
    run([PY, "-s", "tools/slice_audio.py", a.audio, sliced,
         "-34", "4000", "300", "10", "500", "0.9", "0.25", "0", "1"])

    # ---- ASR -> .list (wav|spk|LANG|text) ----
    run([PY, "-s", "tools/asr/funasr_asr.py", "-i", sliced, "-o", base,
         "-s", "large", "-l", a.lang])
    inp_text = "%s/%s.list" % (base, os.path.basename(sliced))
    assert os.path.exists(inp_text), "ASR list not produced: %s" % inp_text

    cvd = str(a.gpu)
    # ---- 1a: text + bert ----
    run([PY, "-s", "GPT_SoVITS/prepare_datasets/1-get-text.py"], env={
        "inp_text": inp_text, "inp_wav_dir": sliced, "exp_name": a.exp,
        "opt_dir": opt_dir, "bert_pretrained_dir": BERT, "is_half": str(is_half),
        "i_part": "0", "all_parts": "1", "_CUDA_VISIBLE_DEVICES": cvd})
    shutil.move("%s/2-name2text-0.txt" % opt_dir, "%s/2-name2text.txt" % opt_dir)

    # ---- 1b: hubert + wav32k, then sv (v2Pro) ----
    b_env = {"inp_text": inp_text, "inp_wav_dir": sliced, "exp_name": a.exp,
             "opt_dir": opt_dir, "cnhubert_base_dir": CNHUBERT, "sv_path": SV_PATH,
             "i_part": "0", "all_parts": "1", "_CUDA_VISIBLE_DEVICES": cvd}
    run([PY, "-s", "GPT_SoVITS/prepare_datasets/2-get-hubert-wav32k.py"], env=b_env)
    run([PY, "-s", "GPT_SoVITS/prepare_datasets/2-get-sv.py"], env=b_env)

    # ---- 1c: semantic ----
    run([PY, "-s", "GPT_SoVITS/prepare_datasets/3-get-semantic.py"], env={
        "inp_text": inp_text, "exp_name": a.exp, "opt_dir": opt_dir,
        "pretrained_s2G": S2G, "s2config_path": S2_CFG,
        "i_part": "0", "all_parts": "1", "_CUDA_VISIBLE_DEVICES": cvd})
    sem = "%s/6-name2semantic.tsv" % opt_dir
    with open(sem, "w", encoding="utf8") as f:
        part = "%s/6-name2semantic-0.tsv" % opt_dir
        f.write("item_name\tsemantic_audio\n" + open(part, encoding="utf8").read())
    os.remove(part)

    # ---- s2 (SoVITS) train ----
    d = json.load(open(S2_CFG))
    d["train"].update({
        "fp16_run": bool(is_half), "batch_size": a.batch if is_half else max(1, a.batch // 2),
        "epochs": a.epochs_s2, "text_low_lr_rate": 0.4, "pretrained_s2G": S2G,
        "pretrained_s2D": S2D, "if_save_latest": True, "if_save_every_weights": True,
        "save_every_epoch": a.epochs_s2, "gpu_numbers": a.gpu, "grad_ckpt": False,
        "lora_rank": 32})
    d["model"]["version"] = VERSION
    d["data"]["exp_dir"] = d["s2_ckpt_dir"] = opt_dir
    d["save_weight_dir"] = wdir
    d["name"] = a.exp
    d["version"] = VERSION
    s2tmp = "%s/tmp_s2.json" % base
    json.dump(d, open(s2tmp, "w"))
    run([PY, "-s", "GPT_SoVITS/s2_train.py", "--config", s2tmp])

    # ---- s1 (GPT) train ----
    import yaml
    y = yaml.load(open(S1_CFG), Loader=yaml.FullLoader)
    if not is_half:
        y["train"]["precision"] = "32"
    y["train"].update({
        "batch_size": a.batch if is_half else max(1, a.batch // 2),
        "epochs": a.epochs_s1, "save_every_n_epoch": a.epochs_s1,
        "if_save_every_weights": True, "if_save_latest": True, "if_dpo": False,
        "half_weights_save_dir": wdir, "exp_name": a.exp})
    y["pretrained_s1"] = S1
    y["train_semantic_path"] = sem
    y["train_phoneme_path"] = "%s/2-name2text.txt" % opt_dir
    y["output_dir"] = "%s/logs_s1_%s" % (opt_dir, VERSION)
    s1tmp = "%s/tmp_s1.yaml" % base
    yaml.dump(y, open(s1tmp, "w"), default_flow_style=False)
    run([PY, "-s", "GPT_SoVITS/s1_train.py", "--config_file", s1tmp],
        env={"_CUDA_VISIBLE_DEVICES": cvd, "hz": "25hz"})

    # ---- locate produced weights ----
    sov = sorted(f for f in os.listdir(wdir) if f.endswith(".pth"))
    gpt = sorted(f for f in os.listdir(wdir) if f.endswith(".ckpt"))
    assert sov and gpt, "training produced no weights in %s" % wdir
    sov_p = "%s/%s" % (wdir, sov[-1])
    gpt_p = "%s/%s" % (wdir, gpt[-1])
    print("\n=== FINE-TUNE DONE ===")
    print("SoVITS:", sov_p)
    print("GPT   :", gpt_p)

    if a.serve_url:
        for ep, q in (("set_sovits_weights", sov_p), ("set_gpt_weights", gpt_p)):
            u = "%s/%s?weights_path=%s" % (a.serve_url.rstrip("/"), ep,
                                           urllib.parse.quote(q))
            print("hot-load:", u)
            print(urllib.request.urlopen(u, timeout=120).read().decode()[:200])
        print("Now POST /tts (ref still required) — voice is the fine-tuned one.")


if __name__ == "__main__":
    import urllib.parse  # noqa
    main()
