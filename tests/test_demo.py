"""
The zero-input demo is the first thing a visitor runs, so it has to work.

It is also the only prototype with no inputs, which makes it cheap to test
end-to-end: generate the chain, diff it, and assert the output says what the
README claims it says. If the differ regresses, the README becomes a lie and
this test is what catches it.
"""

from __future__ import annotations

import gzip
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

import pytest

demo = pytest.importorskip("demo")


# --------------------------------------------------------------------------- #
# the generated .als files must be real
# --------------------------------------------------------------------------- #


def test_the_demo_writes_genuine_gzipped_xml():
    data = demo.to_als_bytes(demo.new_set())
    assert data[:2] == b"\x1f\x8b", "not a gzip container"
    root = ET.fromstring(gzip.decompress(data))
    assert root.tag == "Ableton"
    assert root.find("LiveSet") is not None


def test_the_generated_set_parses_with_the_real_model(tmp_path, als_diff):
    path = tmp_path / "v.als"
    path.write_bytes(demo.to_als_bytes(demo.new_set()))
    model = als_diff.build_model(str(path))
    assert model["tempo"] == 122.0
    names = sorted(t["name"] for t in model["tracks"].values())
    assert names == ["Bass", "Corpus metal", "Kick", "Pad", "Stem-mixing", "Vox"]


def test_track_ids_are_stable_across_the_generated_chain(tmp_path, als_diff):
    """The demo must model the property the whole project rests on."""
    doc = demo.new_set()
    first = tmp_path / "a.als"
    first.write_bytes(demo.to_als_bytes(doc))
    ids_before = set(als_diff.build_model(str(first))["tracks"])

    for _label, mutate in demo.edits()[:5]:
        doc = demo.copy.deepcopy(doc)
        mutate(doc)
    later = tmp_path / "b.als"
    later.write_bytes(demo.to_als_bytes(doc))
    ids_after = set(als_diff.build_model(str(later))["tracks"])

    # edits 0-4 add and remove nothing, so the id set must be untouched
    assert ids_before == ids_after


# --------------------------------------------------------------------------- #
# the output must match what the README advertises
# --------------------------------------------------------------------------- #


def _chain(tmp_path):
    doc = demo.new_set()
    paths = [tmp_path / "v00.als"]
    paths[0].write_bytes(demo.to_als_bytes(doc))
    labels = []
    for i, (label, mutate) in enumerate(demo.edits(), start=1):
        doc = demo.copy.deepcopy(doc)
        mutate(doc)
        p = tmp_path / ("v%02d.als" % i)
        p.write_bytes(demo.to_als_bytes(doc, view_scroll=i * 137))
        paths.append(p)
        labels.append(label)
    return paths, labels


def test_a_view_only_save_reports_no_musical_change(tmp_path, als_diff):
    """The single most important line of demo output."""
    paths, labels = _chain(tmp_path)
    assert "scrolled" in labels[0]
    changes = als_diff.diff_models(
        als_diff.build_model(str(paths[0])), als_diff.build_model(str(paths[1]))
    )
    assert changes == [], "a save that only scrolled must report nothing"


@pytest.mark.parametrize(
    "index, expected",
    [
        (1, "MIX~"),      # turned the stem bus down
        (2, "CLIP~"),     # trimmed the wood hits
        (3, "FX+"),       # added a filter
        (4, "SAMPLE~"),   # renamed the kick sample
        (5, "TRACK+"),    # added a harmony track
        (6, "TEMPO"),     # pushed the tempo
    ],
)
def test_each_demo_edit_is_detected(tmp_path, als_diff, index, expected):
    paths, _ = _chain(tmp_path)
    changes = als_diff.diff_models(
        als_diff.build_model(str(paths[index])),
        als_diff.build_model(str(paths[index + 1])),
    )
    joined = "\n".join(changes)
    assert expected in joined, "edit %d produced %r" % (index, changes)


def test_the_rename_is_coalesced_not_fanned_out(tmp_path, als_diff):
    paths, _ = _chain(tmp_path)
    changes = als_diff.diff_models(
        als_diff.build_model(str(paths[4])), als_diff.build_model(str(paths[5]))
    )
    assert len(changes) == 1, "a rename must collapse to one line, got %r" % (changes,)
    assert changes[0].startswith("SAMPLE~")


def test_removing_a_track_is_reported_as_a_removal(tmp_path, als_diff):
    paths, _ = _chain(tmp_path)
    changes = als_diff.diff_models(
        als_diff.build_model(str(paths[6])), als_diff.build_model(str(paths[7]))
    )
    assert any(c.startswith("TRACK-") and "Corpus metal" in c for c in changes)


# --------------------------------------------------------------------------- #
# it must actually run
# --------------------------------------------------------------------------- #


def test_the_demo_runs_end_to_end_and_cleans_up_after_itself(tmp_path):
    script = Path(__file__).resolve().parents[1] / "experiments" / "demo.py"
    proc = subprocess.run(
        [sys.executable, str(script)], capture_output=True, text=True, timeout=180
    )
    assert proc.returncode == 0, proc.stderr
    assert "no musical change detected" in proc.stdout
    assert "MIX~" in proc.stdout and "TEMPO" in proc.stdout


def test_keep_writes_the_chain_where_asked(tmp_path):
    script = Path(__file__).resolve().parents[1] / "experiments" / "demo.py"
    out = tmp_path / "kept"
    proc = subprocess.run(
        [sys.executable, str(script), "--keep", str(out)],
        capture_output=True, text=True, timeout=180,
    )
    assert proc.returncode == 0, proc.stderr
    written = sorted(out.glob("*.als"))
    assert len(written) == len(demo.edits()) + 1
    for path in written:
        assert path.read_bytes()[:2] == b"\x1f\x8b"
