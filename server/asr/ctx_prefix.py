"""前缀上下文解码的纯逻辑（无模型、无 IO，可单测）。

## 为什么需要它

孤立解码一个短句时，中英混说里的英文词经常被解成汉字音节。实测（toolkit 段
#13340）：说的是「review 一下这个文档」，单独解码得到「伪略一下这个文档。」；
**同一段音频字节不动**，前面拼上一整句无关中文后重解，得到
「你看一下识别记录，review一下这个文档。」——完全正确。

注意拼上去的那句在时间上其实**晚于**本句、内容也毫不相干。所以起作用的不是
「语义上下文」，而是「输入不再是孤零零一小段」。因此拿**上一句已完成句子的音频**
当前缀就够用，不需要它与本句有任何关系。

## 为什么不是「保留最近 N 秒缓冲」

#13340 与它的前一句相隔近 2 分钟。按时间保留缓冲取到的全是静音，对这个案例
毫无用处。所以 app.py 缓存的是**上一句的音频本身**，与静音间隔无关。

## 难点在切，不在拼

识别接口只返回整段字符串，Whisper 侧还显式 `without_timestamps`，没有词级
时间戳可以按时间切。但我们**已经有上一句的识别文本**，用它做文本对齐即可定位
前缀的结束位置——这也是把这段逻辑拆出来能纯文本单测的原因。

对齐不可靠时一律返回 None、退回无上下文的结果：**切错正文比不切更糟**。
"""

import difflib

# 用于定位的匹配块必须覆盖上一句的多大比例。低于此值认为「没找到前缀」。
MIN_PREFIX_COVERAGE = 0.6
# 相邻匹配块在整段文本里的最大允许间隔（字符）。超过即认为后面的匹配是巧合
# ——典型是句末标点跟上一句的句末标点对上了，若不截断会把整条正文一起吃掉。
MAX_BLOCK_GAP = 4
# 切完后要剥掉的前导标点/空白（前缀与本句之间的连接符）。
_LEAD_STRIP = " \t　,，.。!！?？;；:：、-—~～"

# 结果相对「无上下文解码」的长度比。超出即认为前缀没切干净 / 切过头。
LEN_RATIO_MAX = 1.6
LEN_RATIO_MIN = 0.55
# 两者都短于这个长度时不按比例判——几个字的差异就能触发数倍变化。
SHORT_TEXT_CHARS = 8


def strip_known_prefix(full: str, prev: str):
    """从 `full` 里切掉对应 `prev` 的前缀，返回本句文本；定位不到则返回 None。

    `full` 是「上一句音频 + 本句音频」一起解码的整段结果，`prev` 是上一句
    **上次单独解码时**得到的文本。两者不会逐字相同（同一段音频跟别的音频拼在
    一起解码，输出本就会有出入），所以用编辑距离对齐而不是字面前缀匹配。
    """
    if not full or not prev:
        return None

    matcher = difflib.SequenceMatcher(a=prev, b=full, autojunk=False)
    blocks = [b for b in matcher.get_matching_blocks() if b.size > 0]
    if not blocks:
        return None

    # get_matching_blocks 给出的是单调对齐。按序累进，一旦在 full 里出现大跳跃
    # 就停——那之后的匹配已经落进本句正文了。
    cut = 0
    covered = 0
    for block in blocks:
        if block.b - cut > MAX_BLOCK_GAP:
            break
        cut = block.b + block.size
        covered += block.size

    if covered / len(prev) < MIN_PREFIX_COVERAGE:
        return None

    return full[cut:].lstrip(_LEAD_STRIP)


def accept_ctx_text(stripped: str, plain: str) -> bool:
    """带上下文解码 + 切前缀的结果是否可信，不可信则退回 `plain`。

    判据是**长度**而非字面重叠：短文本里整段换写是正常的（「伪略」→「review」
    正是我们要的结果），按重叠度判会把成功的那次一起毙掉。长度暴涨说明前缀
    没切干净，暴跌说明切过头把正文吃了。
    """
    stripped = (stripped or "").strip()
    plain = (plain or "").strip()
    if not stripped:
        return False
    if not plain:
        return True
    if max(len(stripped), len(plain)) < SHORT_TEXT_CHARS:
        return True
    ratio = len(stripped) / len(plain)
    return LEN_RATIO_MIN <= ratio <= LEN_RATIO_MAX
