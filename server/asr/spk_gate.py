"""Speaker gating decision — pure logic, no model/IO, so it can be unit tested.

`finalize()` used to inline this. It is split out because the decision is
where false accepts live (someone else's speech emitted as the enrolled
user's), and that is only debuggable if it can be replayed offline against
recorded cosine scores.

Two things the previous inline version lacked:

* **imposter anchors** — with a single enrolled voiceprint in the library,
  argmax has nowhere else to land: every human voice scores against the one
  person we know, so a stranger only has to clear the threshold. An anchor
  enrolled with role="imposter" gives argmax somewhere else to go, and a
  segment landing on it is dropped instead of emitted.
* **a minimum duration** — embeddings from sub-second slices are noise. In
  the recorded sample set the enrolled user's own 0.94s slice scored 0.3587
  while a stranger's 5.7s slice scored 0.4894: below ~1.2s the score says
  more about the slice length than about who spoke.
"""

ROLE_ENROLLED = "enrolled"
ROLE_IMPOSTER = "imposter"


def best_candidate(candidates):
    """Highest-scoring (name, role, score), or None when there are none."""
    if not candidates:
        return None
    return max(candidates, key=lambda c: c[2])


def scoreboard(candidates):
    """The full "name/role=score" board for the log line.

    Logged on **both** the drop and the emit path, on purpose. The drop case
    needs it to answer "how close was it". The emit case needs it to answer
    the mirror question — "how much margin did this have" — which is just as
    unanswerable after the fact, because scores are not persisted anywhere.

    Without it, a segment that cleared the threshold by 0.01 and one that
    cleared it by 0.30 look identical in the segment table, yet the first is
    a sentence about to start disappearing as soon as the capture path
    shifts slightly (a different mic, audio enhancement toggled, a different
    room). Recorded margins turn that from "one day sentences stopped coming
    through" into something visible before it bites.
    """
    return " ".join(f"{n}/{r}={s:.3f}" for n, r, s in (candidates or []))


def margin(candidates, threshold):
    """Best score minus `threshold`; None when nothing could be scored.

    Negative means nothing in the library cleared the bar. That is still a
    valid emit state when gating is off (reason "unlabeled"), which is
    exactly why the emit path logs the margin rather than assuming it is
    positive.
    """
    best = best_candidate(candidates)
    return None if best is None else best[2] - threshold


def decide(candidates, dur_ms, *, gated, threshold, min_ms):
    """Decide whether to emit a finalized segment and whom to attribute it to.

    `candidates` is [(name, role, score)] already scored against the segment,
    or None when the embedding could not be computed at all.

    Returns (emit, speaker, reason). `speaker` is None whenever we are not
    positively confident, even if the segment is emitted. `reason` is what
    goes in the log line — "imposter" and "unmatched" are kept apart on
    purpose: the first means an anchor claimed the segment, the second that
    nothing in the library did, and they call for different fixes.
    """
    best = best_candidate(candidates)
    matched = bool(best) and best[1] == ROLE_ENROLLED and best[2] >= threshold
    if not gated:
        # Gating off: never drop, only label when positively matched.
        return (True, best[0] if matched else None, "accepted" if matched else "unlabeled")
    if candidates is None:
        return (False, None, "embed_failed")
    if dur_ms < min_ms:
        # Too short for the embedding to carry speaker identity. Under strict
        # gating "cannot judge" resolves to drop, same as "judged and failed" —
        # letting it through is exactly how a stranger's one-word interjection
        # got attributed to the enrolled user.
        return (False, None, "too_short")
    if matched:
        return (True, best[0], "accepted")
    if best and best[1] == ROLE_IMPOSTER and best[2] >= threshold:
        return (False, None, "imposter")
    return (False, None, "unmatched")
