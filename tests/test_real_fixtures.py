"""
Optional tests against real DAW material. Opt in with ``WIT_FIXTURES=/path``.

WHY THESE ARE SEPARATE
    No project file may be committed here, so the rest of the suite runs entirely
    on fixtures this repository generates. Synthetic fixtures prove the code is
    self-consistent. They cannot prove Ableton writes what we think it writes —
    only real material does that, and the whole project is a bet on format
    reverse-engineering being right.

WHY THE SKIP IS LOUD
    The failure mode being avoided is specific and was observed in a neighbouring
    project (logic2ableton): every real-fixture test skipped silently in every
    environment, the suite read as green for months, and a format bug lived
    through all of it. So:

      - conftest prints a red banner at the end of any run where these skipped;
      - ``WIT_REQUIRE_FIXTURES=1`` turns the skip into a failure, which is what a
        nightly job or a release gate should use.

WHAT THEY ASSERT
    Only invariants that hold for *any* real project — never a number specific to
    one person's session. AGENTS.md forbids publishing figures without saying what
    material produced them, and these tests do not know what material they were
    handed.

RULES OBSERVED HERE
    - Read-only. Nothing writes into the material; anything needing a writable
      file copies to tmp_path first, and a fingerprint is checked afterwards.
    - No private data in output. Real sets embed other people's home directories;
      assertion messages go through ``redact()``.

USAGE
    WIT_FIXTURES=~/Music/YourProject/Backup pytest tests/
    WIT_REQUIRE_FIXTURES=1 WIT_FIXTURES=... pytest tests/ -m real_fixtures
"""

from __future__ import annotations

import gzip
import itertools

import pytest

pytestmark = pytest.mark.real_fixtures

MAX_FILES = 12  # keep a 30-save chain from turning into a five-minute test


# --------------------------------------------------------------------------- #
# Ableton .als
# --------------------------------------------------------------------------- #


def test_real_als_files_build_a_model(als_diff, real_material, redactor):
    for path in real_material.als_chain()[:MAX_FILES]:
        model = als_diff.build_model(str(path))
        assert model["tracks"], "no tracks extracted from %s" % redactor(path.name)
        for track in model["tracks"].values():
            assert track["name"], "a track came back with no name"
            assert track["kind"] in als_diff.TRACK_TAGS


def test_real_track_ids_are_stable_across_the_save_chain(als_diff, real_material, redactor):
    """
    docs/EXPERIMENTS.md 2, re-measured on whatever chain is supplied: "Ableton
    track IDs are stable ... the single most important enabling fact for the whole
    project". If it is false for some Live version, diff and merge are impossible
    and this is where we find out.
    """
    files = real_material.als_chain()[:MAX_FILES]
    id_sets = [frozenset(als_diff.build_model(str(p))["tracks"]) for p in files]

    first = id_sets[0]
    churn = [
        (redactor(files[i].name), sorted(first ^ ids)[:5])
        for i, ids in enumerate(id_sets)
        if ids != first
    ]
    # Tracks legitimately get added and removed across a chain, so require a large
    # stable core rather than exact equality.
    common = frozenset.intersection(*id_sets)
    assert len(common) >= 0.5 * len(first), (
        "only %d of %d track Ids survived the whole chain — Ids may be unstable "
        "in this Live version (%s)" % (len(common), len(first), churn[:3])
    )


def line_churn(path_a, path_b):
    """
    Changed lines between two gzipped XML files, counted as a multiset
    difference. `difflib` on a 280,000-line file is quadratic and takes minutes;
    this is a close lower bound on the same quantity and is linear.
    """
    from collections import Counter

    def lines(path):
        with gzip.open(str(path), "rt", encoding="utf-8", errors="replace") as fh:
            return Counter(fh)

    a, b = lines(path_a), lines(path_b)
    return sum((a - b).values()) + sum((b - a).values())


def test_semantic_diff_is_far_quieter_than_a_line_diff(als_diff, real_material, redactor):
    """
    docs/EXPERIMENTS.md 1 and 4 in one assertion: the raw XML diff is dominated by
    noise, and the semantic model removes it. Measured here on the supplied
    material rather than assumed.
    """
    files = real_material.als_chain()[:MAX_FILES]
    ratios = []
    for older, newer in itertools.islice(zip(files, files[1:]), 4):
        raw = line_churn(older, newer)
        semantic = len(
            als_diff.diff_models(
                als_diff.build_model(str(older)), als_diff.build_model(str(newer))
            )
        )
        if raw >= 50:
            ratios.append((raw, semantic))

    if not ratios:
        pytest.skip("no save pair in this material had a substantial raw diff")

    for raw, semantic in ratios:
        assert semantic <= raw, "semantic diff was noisier than the raw line diff"
    assert any(semantic * 5 <= raw for raw, semantic in ratios), (
        "the semantic model did not reduce the noise on any save pair: %s" % ratios
    )


def test_diff_output_never_leaks_a_filesystem_path(als_diff, real_material):
    """
    AGENTS.md treats embedded absolute paths as private data. The model keeps
    sample basenames only, so no diff line may contain a directory separator in a
    path-like position. The reference project contains 777 references to a
    stranger's home directory; none of them should be printable by `wit diff`.
    """
    files = real_material.als_chain()[:MAX_FILES]
    for older, newer in itertools.islice(zip(files, files[1:]), 6):
        for line in als_diff.diff_models(
            als_diff.build_model(str(older)), als_diff.build_model(str(newer))
        ):
            assert "/Users/" not in line and "/home/" not in line, (
                "a diff line exposed an absolute path (redacted): %s"
                % line.replace("/Users/", "/U***/").replace("/home/", "/h***/")
            )


def test_real_material_is_not_modified(als_diff, real_material):
    """AGENTS.md: "Never modify a user's DAW projects. Read-only, always.\""""
    files = real_material.als_chain()[:MAX_FILES]
    before = real_material.fingerprint(files)
    for path in files:
        als_diff.build_model(str(path))
    assert real_material.fingerprint(files) == before


def test_a_copy_is_what_gets_written_to(real_material, tmp_path):
    """The pattern any future writing test must follow."""
    source = real_material.als_chain()[0]
    before = real_material.fingerprint([source])
    scratch = real_material.copy(source, tmp_path / "scratch")
    scratch.write_bytes(scratch.read_bytes() + b"\x00")
    assert real_material.fingerprint([source]) == before


def test_the_semantic_diff_runs_over_the_whole_chain(als_diff, real_material, capsys):
    """Smoke test of the exact command CONTRIBUTING tells a musician to run."""
    files = real_material.als_chain()[:MAX_FILES]
    capsys.readouterr()
    counts = [
        als_diff.report(str(older), str(newer)) for older, newer in zip(files, files[1:])
    ]
    capsys.readouterr()
    assert counts, "no pairs to diff"
    assert all(isinstance(c, int) and c >= 0 for c in counts)


# --------------------------------------------------------------------------- #
# FL Studio .flp
# --------------------------------------------------------------------------- #


def test_real_flp_parses(flp_report, real_material, redactor):
    for path in real_material.flp_files()[:3]:
        report = flp_report(path)
        assert report["events"] > 0, "no events in %s" % redactor(path.name)
        assert report["distinct_ids"] > 0
        assert report["ppq"] > 0
        assert report["file_bytes"] == path.stat().st_size


def test_real_flp_is_dominated_by_opaque_payload(flp_report, real_material):
    """
    docs/EXPERIMENTS.md 10 / FORMATS.md: "92% of the file is opaque
    variable-length plugin state". The exact figure is specific to one file, so
    the assertion is the structural claim — most of a .flp is state Wit can only
    store as a blob — not the number.
    """
    for path in real_material.flp_files()[:3]:
        report = flp_report(path)
        assert 50.0 < report["blob_pct"] <= 100.0, (
            "opaque payload was %.1f%% — if this is now below half, the plugin "
            "wall claim in docs/FORMATS.md needs re-measuring" % report["blob_pct"]
        )


def test_real_flp_text_events_decode_to_something_printable(flp_report, real_material):
    """
    The text heuristic is the weakest part of the FL parser (see
    test_flp_parse.py). On real material, catch the case where it produces CJK
    mojibake from a Latin project — a strong hint that the guess went wrong.
    """
    for path in real_material.flp_files()[:3]:
        report = flp_report(path)
        if not report["texts"]:
            continue
        suspicious = [
            name
            for name, text in report["texts"]
            if text and sum(ord(ch) > 0x2000 for ch in text) > len(text) * 0.5
        ]
        assert not suspicious, (
            "text events decoded to mostly high-codepoint characters (%s); "
            "either this project really is non-Latin, or decode_text guessed "
            "wrong — see the xfails in test_flp_parse.py" % sorted(set(suspicious))
        )


# --------------------------------------------------------------------------- #
# storage
# --------------------------------------------------------------------------- #


def test_chunking_a_real_save_chain_finds_real_reuse(cdc, real_material, capsys):
    """
    Consecutive saves of one project must share most of their content. A number
    is not asserted (that depends entirely on the material); the direction is.
    """
    files = real_material.als_chain()[:4]
    capsys.readouterr()
    cdc.pairwise(str(files[0]), str(files[1]), True)
    out = capsys.readouterr().out
    import re

    reuse = float(re.search(r"reusable from A :\s*([\d.]+)%", out).group(1))
    assert reuse > 0.0, (
        "two consecutive saves of the same project shared no chunks at all — "
        "either the material is not a save chain, or chunking is broken"
    )


def test_logic_project_data_parses_as_a_chunk_chain(cdc, real_material, capsys):
    files = real_material.logic_project_data()[:4]
    capsys.readouterr()
    cdc.pairwise(str(files[0]), str(files[1]))
    out = capsys.readouterr().out
    assert "reusable from A" in out
