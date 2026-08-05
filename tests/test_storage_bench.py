"""
storage_bench.sh — the script behind docs/EXPERIMENTS.md 6c and ADR-0002.

It is the only prototype that shells out, writes files, and runs git, so the
things worth testing are different: that it never touches the material it is
pointed at, that it fails cleanly on bad input, and that its three strategies
come out in the order the design claims.

What is deliberately NOT asserted: the actual ratios. "delta chains are 29x
better" is a measurement on real Ableton history, and AGENTS.md forbids
manufacturing that number from synthetic material. These tests check the
mechanism, not the headline.
"""

from __future__ import annotations

import gzip
import os
import re
import shutil
import subprocess

import pytest
from factories import binary

pytestmark = pytest.mark.slow

REQUIRED_TOOLS = ("zstd", "git", "bc")


def missing_tools():
    return [t for t in REQUIRED_TOOLS if shutil.which(t) is None]


needs_tools = pytest.mark.skipif(
    bool(missing_tools()), reason="needs %s on PATH" % ", ".join(REQUIRED_TOOLS)
)


@pytest.fixture
def bench(repo_root):
    script = repo_root / "experiments" / "storage_bench.sh"
    assert script.exists()

    def _run(src):
        return subprocess.run(
            ["bash", str(script), str(src)], capture_output=True, text=True, timeout=300
        )

    return _run


def make_als_chain(directory, versions=5, records=400):
    """A chain of gzipped XML saves that differ by a few scattered lines each."""
    directory.mkdir(parents=True, exist_ok=True)
    body = binary.xmlish(records, seed=77).decode("utf-8").splitlines()
    for v in range(versions):
        edited = list(body)
        for line_no in (10 * (v + 1), 200 + v, 350 - v):
            edited[line_no] = edited[line_no].replace('Time="', 'Time="9')
        blob = ("\n".join(edited) + "\n").encode("utf-8")
        path = directory / ("Project [2026-05-0%d 12000%d].als" % (v + 1, v))
        with gzip.GzipFile(str(path), "wb", 6, mtime=0) as fh:
            fh.write(blob)
    return sorted(directory.glob("*.als"))


def fingerprint(paths):
    return {str(p): (p.stat().st_size, p.stat().st_mtime_ns) for p in sorted(paths)}


def parse_bench(out):
    def grab(label):
        m = re.search(re.escape(label) + r"\s+([\d.]+) MB", out)
        assert m, "missing %r in:\n%s" % (label, out)
        return float(m.group(1))

    return {
        "naive": grab("1. keep every version"),
        "git": grab("2. git (after gc)"),
        "delta": grab("3. delta chain (zstd)"),
        "versions": int(re.search(r"Versions: (\d+)", out).group(1)),
    }


# --------------------------------------------------------------------------- #
# happy path
# --------------------------------------------------------------------------- #


@needs_tools
def test_ableton_chain_is_measured_and_delta_wins(bench, tmp_path):
    src = tmp_path / "Backup"
    make_als_chain(src, versions=5)

    proc = bench(src)
    assert proc.returncode == 0, proc.stderr

    assert "Material: Ableton .als" in proc.stdout
    result = parse_bench(proc.stdout)
    assert result["versions"] == 5
    assert result["delta"] < result["naive"], "delta chain must beat keeping every copy"
    assert result["delta"] <= result["git"], (
        "delta chain lost to git on near-identical saves; that is the one result "
        "ADR-0002 depends on"
    )
    assert "<-- Wit" in proc.stdout
    assert "average cost per save" in proc.stdout


@needs_tools
def test_logic_project_data_chain_is_recognised(bench, tmp_path):
    src = tmp_path / "Project File Backups"
    base = binary.incompressible(60_000, seed=81)
    for v in range(3):
        d = src / ("0%d" % v)
        d.mkdir(parents=True)
        (d / "ProjectData").write_bytes(
            binary.replace_region(base, 10_000 * (v + 1), b"\x99" * 200)
        )

    proc = bench(src)
    assert proc.returncode == 0, proc.stderr
    assert "Material: Logic ProjectData" in proc.stdout
    assert parse_bench(proc.stdout)["versions"] == 3


@needs_tools
def test_source_material_is_never_modified(bench, tmp_path):
    """
    AGENTS.md: "Never modify a user's DAW projects. Read-only, always." The script
    is pointed straight at somebody's Backup folder, so this is the one that
    matters most.
    """
    src = tmp_path / "Backup"
    files = make_als_chain(src, versions=4)
    before = fingerprint(files)
    before_listing = sorted(os.listdir(src))

    proc = bench(src)
    assert proc.returncode == 0, proc.stderr

    assert fingerprint(sorted(src.glob("*.als"))) == before
    assert sorted(os.listdir(src)) == before_listing, "the script created files in SRC"


@needs_tools
def test_a_source_path_containing_spaces_works(bench, tmp_path):
    """Real Ableton paths look like '.../Artefakt - Undertow Project/Backup'."""
    src = tmp_path / "Artefakt - Undertow Project" / "Backup"
    make_als_chain(src, versions=3)
    proc = bench(src)
    assert proc.returncode == 0, proc.stderr
    assert parse_bench(proc.stdout)["versions"] == 3


@needs_tools
def test_temporary_working_directory_is_cleaned_up(bench, tmp_path, monkeypatch):
    src = tmp_path / "Backup"
    make_als_chain(src, versions=3)
    workdir = tmp_path / "tmpdir"
    workdir.mkdir()
    monkeypatch.setenv("TMPDIR", str(workdir))

    proc = bench(src)
    assert proc.returncode == 0, proc.stderr
    assert list(workdir.iterdir()) == [], "the trap did not remove the scratch dir"


# --------------------------------------------------------------------------- #
# failure paths
# --------------------------------------------------------------------------- #


@needs_tools
def test_no_material_is_a_clean_error(bench, tmp_path):
    empty = tmp_path / "empty"
    empty.mkdir()
    proc = bench(empty)
    assert proc.returncode == 1
    assert "No *.als or */ProjectData found" in proc.stdout


@needs_tools
def test_a_single_version_is_rejected(bench, tmp_path):
    src = tmp_path / "Backup"
    make_als_chain(src, versions=1)
    proc = bench(src)
    assert proc.returncode != 0
    assert "need >= 2 versions" in proc.stdout


def test_missing_argument_is_rejected(repo_root):
    proc = subprocess.run(
        ["bash", str(repo_root / "experiments" / "storage_bench.sh")],
        capture_output=True,
        text=True,
    )
    assert proc.returncode != 0
    assert "usage: storage_bench.sh" in proc.stderr


@needs_tools
def test_a_corrupt_als_fails_loudly_rather_than_reporting_a_number(bench, tmp_path):
    """
    gunzip fails on the bad file. With `set -euo pipefail` that must abort — a
    benchmark that silently drops a version would publish a wrong figure, which
    AGENTS.md treats as the most serious kind of mistake here.
    """
    src = tmp_path / "Backup"
    make_als_chain(src, versions=3)
    (src / "broken.als").write_bytes(b"not gzip")

    proc = bench(src)
    assert proc.returncode != 0
    assert "MB    <-- Wit" not in proc.stdout


# --------------------------------------------------------------------------- #
# lint
# --------------------------------------------------------------------------- #


@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="shellcheck not installed")
def test_shellcheck_is_clean(repo_root):
    """CI runs this too; having it here shortens the loop to seconds."""
    proc = subprocess.run(
        ["shellcheck", str(repo_root / "experiments" / "storage_bench.sh")],
        capture_output=True,
        text=True,
    )
    assert proc.returncode == 0, proc.stdout
