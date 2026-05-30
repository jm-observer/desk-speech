# tts-voices — 本地音色 / bake-off 暂存区

CosyVoice2 bake-off 的本地工作目录，位于 `server/tts/tts-voices/`。**不是服务端
挂载目录**——线上服务读的是 GB10 上的 `~/tts-voices/`（见同级 `compose.tts.yaml`
的 `:/voices:ro` 挂载，绝对路径，与本 repo 布局解耦）。这里是给人用的素材/产物暂存区。
`refs/` 与脚本可入 git，`outputs/` 已在 `.gitignore` 忽略。

## 布局

```
tts-voices/
├── refs/                     # 参考音色（zero-shot prompt 用的 ref wav）
│   ├── zh/                   #   中文：edge_*（Edge TTS, dev-only）+ cosy_*（Apache-2.0）
│   │   ├── voices.json       #   中文音色清单（服务端契约：扁平 file 名）
│   │   └── *.wav
│   └── en/                   #   英文：LibriTTS-R（CC BY 4.0），file == id 命名
│       ├── en_voices.json    #   英文音色清单（每个音色带 prompt_text_override）
│       ├── transcripts.{json,txt}
│       ├── _SPEAKERS.TXT     #   LibriSpeech reader→性别 表（gen 脚本数据源）
│       └── en_<f|m>_<reader>.wav
├── outputs/                  # bake-off 试听产物（不是 ref，仅供主观对比，gitignored）
│   ├── api_smoke/  emotion_grid/  instruct_grid/  instruct_grid_v2/  voice_lib_test/
│   └── en_ref_clone/         # syn_en_ref_clone.py 的输出（ref × 句子矩阵）
├── en_sentences.{json,txt}   # 英文测试句子集（syn_en_ref_clone.py 输入）
├── gen_edge_refs.py          # 生成 refs/zh/（Edge TTS）
├── gen_en_refs.py            # 生成 refs/en/（LibriTTS-R 流式挑选 + 质量过滤）
└── syn_en_ref_clone.py       # 英文 ref × 句子矩阵合成（调 :8095 /tts/zero_shot）
```

## 音色清单 schema

`voices.json` / `en_voices.json` 同款，字段：`id` `file` `gender` `tone`
`source` `license` `prompt_text`(顶层默认) `prompt_text_override`(逐音色覆盖)。
服务端按 `file`（裸文件名）从挂载目录加载 wav，按 `prompt_text[_override]`
作 zero-shot 的 prompt 文本。

## 部署到服务端

服务端要求 `/voices` 下**扁平**布局（`voices.json` + 同级 `*.wav`）。中文那套
直接 `scp refs/zh/* fengqi@192.168.0.68:~/tts-voices/` 即可。英文 ref 目前仅本地
实验，未并入服务端音色库（并入需把 en wav + 合并 entry 推到 `~/tts-voices/`）。

## 复现

```bash
python gen_edge_refs.py            # 重建 refs/zh/（需 edge-tts、ffmpeg）
python gen_en_refs.py              # 重建 refs/en/（需 datasets、huggingface_hub）
python syn_en_ref_clone.py         # 跑英文 ref × 句子矩阵 → outputs/en_ref_clone/
```
注：`refs/en/` 为人工策展集，重跑会产出等价干净但可能不同的 reader。
`syn_en_ref_clone.py` 默认只用 `en_f_84` + `en_m_2803`（1F+1M），加 `--all-voices`
跑全部 6 个；需 TTS 服务在 `:8095` 监听。
