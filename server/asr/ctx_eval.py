"""前缀上下文解码的离线评测（P0-b）。

判据与理由见 `docs/asr-ctx-prefix-postmortem.md`。这里只做**离线**部分：
筛样本、按当前闸门重新分类、聚合人工标注、算通过线。重解需要 GPU 模型，
由 `--redecode` 走同机 `/transcribe`，不在本模块的纯逻辑里。

## 两个用法

- **改了闸门要重新验收** → `reclassify`。`full` 已经存在观测记录里，
  而 `strip_known_prefix` / `accept_ctx_text` 都是纯文本函数，所以对**全部**记录
  重新分类是零解码成本的。这一步必须跑，因为闸门一改样本集合就变了：放宽
  `accept_ctx_text` 会让旧的 `rejected` 变成新的 `would_apply`，动
  `MIN_PREFIX_COVERAGE`/`MAX_BLOCK_GAP` 会让旧的 `not_located` 变成新的
  `would_apply`——**那批新放行的样本此前从未被听审过，正是最需要评估的**。
- **算通过线** → `verdict`，吃一份人工标注（good/bad/neutral/skip）。

## 为什么分母只取 would_apply

只有它会真的改动用户看到的文本，也只有它能做「无上下文 vs 带上下文」的二选一。
`not_located` 根本没有候选文本（切分返回 None，存下的 `full` 是"上一句+本句"的
整段转写，不能与 `plain` 二选一）；`rejected`/`unchanged` 不改输出。
把它们塞进分母只会**稀释风险**。
"""

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path

from ctx_observe import sample_is_complete
from ctx_prefix import accept_ctx_text, strip_known_prefix

# 通过线三条，缺一不可。只看「0 坏、≥5 好」，有效样本缩水到十几条照样通过。
MIN_VALID = 60
MIN_GOOD = 5


def load_records(root) -> list[dict]:
    """读观测目录下所有 JSONL 行。坏行跳过并计数（不让一行损坏毁掉整批）。"""
    out: list[dict] = []
    for f in sorted(Path(root).rglob("*.jsonl")):
        for line in f.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                print(f"[ctx-eval] 跳过坏行：{f}", file=sys.stderr)
    return out


def is_candidate(record: dict, root) -> bool:
    """能不能进评测分母。三个条件同时满足才算。"""
    if record.get("stage") != "would_apply":
        return False
    if not record.get("would_emit_if_enabled"):
        return False
    # 完整性要实际读一遍 wav 校验帧数——「文件存在」不够，崩溃会留下存在但截断的文件。
    return sample_is_complete(record, root)


def reclassify(record: dict) -> str:
    """用**当前**闸门对一条记录重新分类，不解码。

    输入用的是记录里存下来的 `full`（那次重解的整段结果）、`prev_text`、`plain`。
    返回的 stage 与 app.py 的取值一致。
    """
    full = (record.get("full") or "").strip()
    prev = record.get("prev_text") or ""
    plain = record.get("plain") or ""
    if not full:
        return record.get("stage") or "unknown"  # 压根没跑重解的那几类，原样返回
    stripped = strip_known_prefix(full, prev)
    if stripped is None:
        return "not_located"
    if not accept_ctx_text(stripped, plain):
        return "rejected"
    if stripped == plain:
        return "unchanged"
    return "would_apply"


@dataclass
class Verdict:
    valid: int
    good: int
    bad: int
    neutral: int

    @property
    def passed(self) -> bool:
        return self.valid >= MIN_VALID and self.bad == 0 and self.good >= MIN_GOOD

    def render(self) -> str:
        mark = "PASS" if self.passed else "FAIL"
        return (
            f"有效样本 {self.valid}（需 ≥{MIN_VALID}）  "
            f"改好 {self.good}（需 ≥{MIN_GOOD}）  "
            f"改坏 {self.bad}（需 =0）  无差别 {self.neutral}  → {mark}"
        )


def aggregate(labels: dict[str, list[str]]) -> Verdict:
    """把「每个样本若干轮的人工标注」聚合成结论。

    每轮取值：`good` / `bad` / `neutral` / `skip`（该轮没落到 would_apply，即没改写）。

    - **非改写轮既不计好也不计坏**——它不会改动用户看到的东西。
    - **至少一轮改写才算有效样本**，否则移出分母：这套闸门实际不会动它。
    - **按样本取最坏结果**：有效样本里只要有一轮判坏，该样本即计坏。线上每次改写
      独立发生，多数表决会把 1/3 概率的坏结果洗掉。
    """
    valid = good = bad = neutral = 0
    for rounds in labels.values():
        rewrote = [r for r in rounds if r != "skip"]
        if not rewrote:
            continue
        valid += 1
        if "bad" in rewrote:
            bad += 1
        elif "good" in rewrote:
            good += 1
        else:
            neutral += 1
    return Verdict(valid=valid, good=good, bad=bad, neutral=neutral)


# ---- CLI ---------------------------------------------------------------


def _cmd_reclassify(args) -> int:
    recs = load_records(args.root)
    if not recs:
        print("观测目录里没有记录——影子模式开了吗？", file=sys.stderr)
        return 2
    moved: dict[tuple, int] = {}
    now_candidates = 0
    for r in recs:
        old, new = r.get("stage"), reclassify(r)
        moved[(old, new)] = moved.get((old, new), 0) + 1
        if new == "would_apply" and r.get("would_emit_if_enabled"):
            now_candidates += 1
    print(f"记录总数 {len(recs)}")
    print("\n采集时 → 当前闸门：")
    for (old, new), n in sorted(moved.items(), key=lambda kv: -kv[1]):
        flag = "  ← 新放行，必须补听审" if old != "would_apply" and new == "would_apply" else ""
        print(f"  {old:>14} → {new:<14} {n:>4}{flag}")
    print(f"\n当前闸门下的候选（would_apply 且会发出）：{now_candidates}"
          f"（通过线要求 ≥{MIN_VALID}）")
    return 0


def _cmd_worklist(args) -> int:
    """产出待听审清单：每条给出 wav 路径、无上下文结果、带上下文结果。"""
    root = Path(args.root)
    rows = [r for r in load_records(root) if is_candidate(r, root)]
    for r in rows:
        print(json.dumps({
            "key": r["key"],
            "wav_plain": r.get("wav_plain"),
            "wav_ctx": r.get("wav_ctx"),
            "plain": r.get("plain"),
            "stripped": r.get("stripped"),
        }, ensure_ascii=False))
    print(f"\n共 {len(rows)} 条待听审。标注写成 {{key: [good|bad|neutral|skip, ...]}} 的 JSON，"
          f"再跑 verdict。", file=sys.stderr)
    return 0


def _cmd_verdict(args) -> int:
    labels = json.loads(Path(args.labels).read_text(encoding="utf-8"))
    v = aggregate(labels)
    print(v.render())
    return 0 if v.passed else 1


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("reclassify", help="用当前闸门对全部记录重新分类（不解码）")
    p.add_argument("root")
    p.set_defaults(fn=_cmd_reclassify)

    p = sub.add_parser("worklist", help="产出待听审清单")
    p.add_argument("root")
    p.set_defaults(fn=_cmd_worklist)

    p = sub.add_parser("verdict", help="吃人工标注，算通过线")
    p.add_argument("labels")
    p.set_defaults(fn=_cmd_verdict)

    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
