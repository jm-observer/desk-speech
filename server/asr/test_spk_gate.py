"""Replay of the recorded speaker-gating failure.

The scores below are real: 17 slices captured by zero-desktop's annotation
flow, embedded on GB10 (`:9101/embed`) and scored against the live voiceprint
library. Samples 42-45 are four slices the user explicitly marked
`speaker_wrong` — someone else's speech that the gate attributed to the
enrolled user and emitted. `cos_imposter` for those four is leave-one-out
(the sample itself is excluded from the anchor set), otherwise a sample would
score 1.0 against itself and the anchors would look better than they are.
"""
import pytest

from spk_gate import ROLE_ENROLLED, ROLE_IMPOSTER, decide

# (sample_id, truth, dur_ms, cos_fengqi, cos_imposter)
SAMPLES = [
    (1, "fengqi", 28540, 0.6953, 0.3457),
    (2, "fengqi", 37490, 0.6287, 0.4130),
    (3, "fengqi", 10390, 0.6638, 0.4005),
    (4, "fengqi", 35560, 0.6763, 0.3761),
    (5, "fengqi", 48840, 0.7085, 0.3693),
    (6, "fengqi", 5900, 0.7506, 0.4088),
    (19, "fengqi", 1190, 0.3722, 0.3611),
    (21, "fengqi", 940, 0.3587, 0.4802),
    (23, "fengqi", 2370, 0.4962, 0.2819),
    (24, "fengqi", 2370, 0.4962, 0.2819),
    (29, "fengqi", 27740, 0.7678, 0.4227),
    (32, "fengqi", 6820, 0.5792, 0.3594),
    (41, "fengqi", 6000, 0.7591, 0.4096),
    (42, "imposter", 5720, 0.4894, 0.7375),
    (43, "imposter", 1140, 0.3681, 0.7179),
    (44, "imposter", 3330, 0.3715, 0.7375),
    (45, "imposter", 4880, 0.3535, 0.6636),
]

IMPOSTERS = [s for s in SAMPLES if s[1] == "imposter"]
# The enrolled user's own slices that are long enough for the embedding to
# mean anything. The sub-1.2s ones (19, 21) are excluded on purpose: see
# MIN_MS below — nothing can rescue them, they are dropped by design.
MINE_USABLE = [s for s in SAMPLES if s[1] == "fengqi" and s[2] >= 1200]

THRESHOLD = 0.45
MIN_MS = 1200


def candidates(cos_fengqi, cos_imposter, *, with_anchor):
    c = [("fengqi", ROLE_ENROLLED, cos_fengqi)]
    if with_anchor:
        c.append(("imposter-1", ROLE_IMPOSTER, cos_imposter))
    return c


@pytest.mark.parametrize("sid,_truth,dur,fq,imp", IMPOSTERS)
def test_imposter_slices_are_dropped(sid, _truth, dur, fq, imp):
    """The four slices the user marked speaker_wrong must not be emitted."""
    emit, speaker, reason = decide(
        candidates(fq, imp, with_anchor=True), dur,
        gated=True, threshold=THRESHOLD, min_ms=MIN_MS,
    )
    assert not emit, f"sample {sid} still emitted as {speaker!r} ({reason})"
    assert speaker is None


@pytest.mark.parametrize("sid,_truth,dur,fq,imp", MINE_USABLE)
def test_enrolled_speech_still_accepted(sid, _truth, dur, fq, imp):
    """Rejecting the stranger must not cost the enrolled user their own speech."""
    emit, speaker, reason = decide(
        candidates(fq, imp, with_anchor=True), dur,
        gated=True, threshold=THRESHOLD, min_ms=MIN_MS,
    )
    assert emit, f"sample {sid} dropped ({reason})"
    assert speaker == "fengqi"


def test_slices_too_short_to_judge_are_dropped_when_gated():
    """Sub-threshold-duration slices are unjudgeable, not 'probably fine'."""
    emit, speaker, reason = decide(
        candidates(0.9, 0.1, with_anchor=True), MIN_MS - 1,
        gated=True, threshold=THRESHOLD, min_ms=MIN_MS,
    )
    assert (emit, speaker, reason) == (False, None, "too_short")


def test_imposter_hit_is_reported_distinctly_from_plain_mismatch():
    """The log has to say *why*: a matched anchor is not the same as noise."""
    _, _, hit = decide(
        candidates(0.30, 0.70, with_anchor=True), 5000,
        gated=True, threshold=THRESHOLD, min_ms=MIN_MS,
    )
    _, _, noise = decide(
        candidates(0.20, 0.15, with_anchor=True), 5000,
        gated=True, threshold=THRESHOLD, min_ms=MIN_MS,
    )
    assert hit == "imposter"
    assert noise == "unmatched"


def test_embed_failure_drops_when_gated():
    assert decide(None, 5000, gated=True, threshold=THRESHOLD, min_ms=MIN_MS) == (
        False, None, "embed_failed",
    )


def test_gating_off_never_drops():
    """With gating off the gate only labels; dropping is not its business."""
    for _sid, _t, dur, fq, imp in SAMPLES:
        emit, _speaker, _reason = decide(
            candidates(fq, imp, with_anchor=True), dur,
            gated=False, threshold=THRESHOLD, min_ms=MIN_MS,
        )
        assert emit
    emit, speaker, _ = decide(None, 100, gated=False, threshold=THRESHOLD, min_ms=MIN_MS)
    assert emit and speaker is None


def test_single_voiceprint_library_cannot_reject_the_stranger():
    """Why an anchor, and not just a higher threshold.

    With only the enrolled user in the library argmax has nowhere else to
    land, so rejecting the stranger means outrunning him on a single number —
    and he gets within 0.01 of the enrolled user's own short slices. This
    test pins the shape of that failure; it is the justification for the
    anchor, so it keeps asserting the *unanchored* behaviour after the fix.
    """
    def accepted(threshold, group):
        return [
            sid for sid, _t, dur, fq, imp in group
            if decide(candidates(fq, imp, with_anchor=False), dur,
                      gated=True, threshold=threshold, min_ms=0)[0]
        ]

    # Production threshold: all four of the stranger's slices got through.
    assert accepted(0.35, IMPOSTERS) == [42, 43, 44, 45]

    # The lowest threshold that rejects all four (his best is 0.4894) already
    # costs the enrolled user his own short slices — and leaves 0.01 of
    # headroom, which is no headroom at all. Push it one step further, to
    # 0.50, and two more of his slices go with it.
    assert accepted(0.49, IMPOSTERS) == []
    def mine_lost(threshold):
        return [
            sid for sid, _t, dur, fq, imp in SAMPLES
            if _t == "fengqi" and not decide(
                candidates(fq, imp, with_anchor=False), dur,
                gated=True, threshold=threshold, min_ms=0)[0]
        ]
    assert mine_lost(0.49) == [19, 21]
    assert mine_lost(0.50) == [19, 21, 23, 24]

    # With the anchor the same four are rejected by a 0.25 margin instead.
    for sid, _t, dur, fq, imp in IMPOSTERS:
        assert imp - fq > 0.20, f"sample {sid} margin too thin"
