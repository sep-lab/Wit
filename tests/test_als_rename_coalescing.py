"""
Sample-rename coalescing — the highest-risk logic in the differ.

docs/EXPERIMENTS.md 4 leads with this: a save whose raw diff was 425 clip-level
changes collapses to three semantic lines, two of which are ``SAMPLE~`` renames.
The collapse is what makes the output readable. But the detector is a heuristic:

    "clip C referenced X before and references Y now" => "X was renamed to Y"

That inference is only sound when *every* reference moved and nothing still points
at the old name. When it is unsound it does not fail quietly — it prints a
confident sentence about a file rename that never happened.

These tests cover both sides: that a genuine rename really is coalesced, and that
things which are not renames are not reported as renames.
"""

from __future__ import annotations

import os
import subprocess
import sys
import textwrap

from factories.als import Clip, LiveSet, Track


def sample_lines(lines):
    return [ln for ln in lines if ln.startswith("SAMPLE~")]


def set_with(*specs):
    """set_with(("100", [("10", "a.wav"), ...]), ...) -> LiveSet"""
    tracks = []
    for tid, clips in specs:
        tracks.append(
            Track(
                id=tid,
                name="Track %s" % tid,
                clips=[
                    Clip(id=cid, name="clip %s" % cid, start=float(n), end=float(n) + 4,
                         sample="Samples/Imported/%s" % sample)
                    for n, (cid, sample) in enumerate(clips)
                ],
            )
        )
    return LiveSet(tracks=tracks)


# --------------------------------------------------------------------------- #
# the case coalescing exists for
# --------------------------------------------------------------------------- #


def test_a_real_rename_collapses_to_one_line(diff_of):
    """
    Every reference to the old name moved, and the old name is gone. That is a
    rename, and 6 clip changes must become 1 line.
    """
    a = set_with(
        ("100", [("1", "old.wav"), ("2", "old.wav"), ("3", "old.wav")]),
        ("101", [("1", "old.wav"), ("2", "old.wav"), ("3", "old.wav")]),
    )
    b = set_with(
        ("100", [("1", "new.wav"), ("2", "new.wav"), ("3", "new.wav")]),
        ("101", [("1", "new.wav"), ("2", "new.wav"), ("3", "new.wav")]),
    )
    assert diff_of(a, b) == ["SAMPLE~ 'old.wav' -> 'new.wav'  (6 clip reference(s))"]


def test_a_rename_produces_no_clip_level_noise(diff_of):
    """The point of the collapse: 425 lines -> 1. No CLIP~ fan-out."""
    a = set_with(("100", [(str(i), "old.wav") for i in range(1, 41)]))
    b = set_with(("100", [(str(i), "new.wav") for i in range(1, 41)]))
    lines = diff_of(a, b)
    assert len(lines) == 1
    assert lines[0].endswith("(40 clip reference(s))")


def test_two_independent_renames_are_ordered_by_reference_count(diff_of):
    """docs/EXPERIMENTS.md 4 shows 418 references listed above 6."""
    a = set_with(
        ("100", [(str(i), "big.wav") for i in range(1, 6)] + [("9", "small.wav")]),
    )
    b = set_with(
        ("100", [(str(i), "big2.wav") for i in range(1, 6)] + [("9", "small2.wav")]),
    )
    lines = sample_lines(diff_of(a, b))
    assert lines == [
        "SAMPLE~ 'big.wav' -> 'big2.wav'  (5 clip reference(s))",
        "SAMPLE~ 'small.wav' -> 'small2.wav'  (1 clip reference(s))",
    ]


# --------------------------------------------------------------------------- #
# things that are not renames
# --------------------------------------------------------------------------- #


def test_adding_a_sample_to_an_empty_clip_is_not_a_rename(diff_of):
    a = set_with(("100", [("1", "")]))
    b = set_with(("100", [("1", "kick.wav")]))
    assert sample_lines(diff_of(a, b)) == []


def test_clearing_a_sample_is_not_a_rename(diff_of):
    a = set_with(("100", [("1", "kick.wav")]))
    b = set_with(("100", [("1", "")]))
    assert sample_lines(diff_of(a, b)) == []


def test_a_swap_is_not_reported_as_two_bogus_renames(diff_of):
    """
    Fixed: this used to characterise today's (buggy) behaviour — two renames
    reported in opposite directions for a same-set swap, which is not a thing
    that can happen on a filesystem. See the test below for the full story.
    """
    a = set_with(("100", [("1", "kick.wav"), ("2", "snare.wav")]))
    b = set_with(("100", [("1", "snare.wav"), ("2", "kick.wav")]))
    assert len(sample_lines(diff_of(a, b))) == 0


def test_swapping_two_samples_between_clips_is_not_a_rename(diff_of):
    a = set_with(("100", [("1", "kick.wav"), ("2", "snare.wav")]))
    b = set_with(("100", [("1", "snare.wav"), ("2", "kick.wav")]))

    lines = diff_of(a, b)
    assert sample_lines(lines) == [], (
        "both samples still exist and are still used; nothing was renamed, yet "
        "the diff says: %s" % sample_lines(lines)
    )


def test_a_partial_move_while_the_old_sample_is_still_in_use_is_not_a_rename(diff_of):
    """
    Fixed: same root cause as the swap case. Only 3 of 5 references moved and
    'old.wav' is still used by the other 2, so it demonstrably was not renamed
    — the producer replaced the sample on three clips. diff_models now
    consults model['samples'] before calling anything a rename.
    """
    a = set_with(("100", [(str(i), "old.wav") for i in range(1, 6)]))
    b = set_with(
        ("100", [("1", "new.wav"), ("2", "new.wav"), ("3", "new.wav"),
                 ("4", "old.wav"), ("5", "old.wav")])
    )
    assert sample_lines(diff_of(a, b)) == []


def test_a_rename_storm_no_longer_crowds_out_real_changes(als_diff, capsys, tmp_path):
    """
    Fixed: this used to characterise the false positives crowding out a real
    edit under --limit — report() truncates, and the (bogus) renames were
    emitted first. A 12-way rotation is a permutation of the same 12 names, so
    every "old" name is still in use somewhere in b; none of it is a rename
    any more, and the one edit a musician would care about is no longer
    evicted.
    """
    a = set_with(("100", [(str(i), "s%d.wav" % i) for i in range(1, 13)]))
    b = a.copy()
    for i, clip in enumerate(b.tracks[0].clips):
        clip.sample = "Samples/Imported/s%d.wav" % ((i + 1) % 12 + 1)  # rotate
    b.tracks[0].volume = 0.5  # the one edit a musician would care about

    pa = str(tmp_path / "a.als")
    pb = str(tmp_path / "b.als")
    a.write(pa)
    b.write(pb)

    capsys.readouterr()
    als_diff.report(pa, pb, limit=5)
    out = capsys.readouterr().out

    assert "volume" in out
    assert "SAMPLE~" not in out
    assert "... and" not in out


# --------------------------------------------------------------------------- #
# coverage gaps in the detector
# --------------------------------------------------------------------------- #


def test_samples_on_added_or_removed_tracks_are_never_considered(diff_of):
    """
    Rename detection only looks at tracks present in both versions. A rename that
    coincides with a track being added is invisible to it. Characterisation.
    """
    a = set_with(("100", [("1", "old.wav")]))
    b = set_with(("100", [("1", "new.wav")]), ("101", [("1", "new.wav")]))
    lines = diff_of(a, b)
    assert "SAMPLE~ 'old.wav' -> 'new.wav'  (1 clip reference(s))" in lines
    assert sum(1 for ln in lines if ln.startswith("SAMPLE~")) == 1


def test_renaming_a_sample_used_by_a_clip_that_also_moved_reports_both(diff_of):
    a = set_with(("100", [("1", "old.wav")]))
    b = a.copy()
    clip = b.tracks[0].clips[0]
    clip.sample = "Samples/Imported/new.wav"
    clip.start, clip.end = 8.0, 12.0
    assert diff_of(a, b) == [
        "SAMPLE~ 'old.wav' -> 'new.wav'  (1 clip reference(s))",
        "CLIP~   [Track 100] 'clip 1' 0.0-4.0 -> 8.0-12.0",
    ]


# --------------------------------------------------------------------------- #
# output stability
# --------------------------------------------------------------------------- #


def test_diff_output_order_is_stable_across_processes(tmp_path):
    """
    Fixed: the rename block used to iterate set intersections directly, so the
    insertion order of the `renames` dict followed Python's randomised string
    hashing, and `sorted(..., key=lambda kv: -kv[1])` is a stable sort — ties
    came out in a different order on every process. The sort key is now
    `(-count, old, new)`, so ties break on the sample names instead of on
    iteration order.
    """
    a = set_with(*[("%d" % (200 + i), [("1", "s%d.wav" % i)]) for i in range(8)])
    b = set_with(*[("%d" % (200 + i), [("1", "r%d.wav" % i)]) for i in range(8)])
    pa, pb = str(tmp_path / "a.als"), str(tmp_path / "b.als")
    a.write(pa)
    b.write(pb)

    script = textwrap.dedent(
        """
        import sys
        sys.path.insert(0, sys.argv[1])
        import als_semantic_diff as A
        print("|".join(A.diff_models(A.build_model(sys.argv[2]), A.build_model(sys.argv[3]))))
        """
    )
    experiments = os.path.join(os.path.dirname(os.path.dirname(__file__)), "experiments")
    results = set()
    for seed in ("0", "1", "2", "3", "4", "5"):
        env = dict(os.environ)
        env["PYTHONHASHSEED"] = seed
        proc = subprocess.run(
            [sys.executable, "-c", script, experiments, pa, pb],
            capture_output=True, text=True, check=True, env=env,
        )
        results.add(proc.stdout.strip())

    assert len(results) == 1, (
        "the same two files produced %d different diff orderings across processes"
        % len(results)
    )
