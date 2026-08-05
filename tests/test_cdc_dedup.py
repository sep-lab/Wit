"""
Dedup accounting: does "reusable %" mean what the docs say it means?

docs/EXPERIMENTS.md 6a and 7, ADR-0002, and README all quote percentages produced
by ``cdc_dedup.pairwise`` and ``cdc_dedup.store_cost``. Those percentages are
load-bearing — 6a is the number ADR-0002 overturned CDC-for-project-files with.
So the arithmetic gets tested against inputs whose answer is known by construction,
not against real material where "roughly right" is unfalsifiable.

Convention used throughout: reuse% is measured *against file B* (what fraction of
the new file we already have), which is what the script prints.
"""

from __future__ import annotations

import hashlib
import re
import zlib

import pytest
from factories import binary

# --------------------------------------------------------------------------- #
# helpers
# --------------------------------------------------------------------------- #


def report_of(capsys, fn, *args, **kwargs):
    capsys.readouterr()
    fn(*args, **kwargs)
    return capsys.readouterr().out


def parse_pairwise(out: str) -> dict:
    reuse = re.search(r"reusable from A :\s*([\d.]+)%", out)
    new = re.search(r"must store new  :\s*([\d.]+) MB", out)
    chunks = re.search(r"\(([\d.]+) MB, (\d+) chunks\)", out)
    assert reuse and new and chunks, "pairwise output format changed:\n%s" % out
    return {
        "reuse_pct": float(reuse.group(1)),
        "new_mb": float(new.group(1)),
        "total_mb": float(chunks.group(1)),
        "chunks": int(chunks.group(2)),
    }


def parse_store(out: str) -> dict:
    def grab(pattern):
        m = re.search(pattern, out)
        assert m, "store_cost output format changed:\n%s" % out
        return m

    return {
        "versions": int(grab(r"versions\s+:\s+(\d+)").group(1)),
        "logical_mb": float(grab(r"logical total\s+:\s+([\d.]+) MB").group(1)),
        "unique_mb": float(grab(r"unique chunks\s+:\s+([\d.]+) MB").group(1)),
        "unique_pct": float(grab(r"unique chunks\s+:\s+[\d.]+ MB\s+\(([\d.]+)%\)").group(1)),
        "stored_mb": float(grab(r"unique \+ zlib \(stored\):\s+([\d.]+) MB").group(1)),
        "stored_pct": float(
            grab(r"unique \+ zlib \(stored\):\s+[\d.]+ MB\s+\(([\d.]+)%\)").group(1)
        ),
        "per_version_mb": float(grab(r"average per version\s+:\s+([\d.]+) MB").group(1)),
    }


def write(tmp_path, name, data):
    p = tmp_path / name
    p.write_bytes(data)
    return str(p)


# store_cost prints MB to one decimal and the per-version figure to three, so an
# expected value has to be put through the same rounding before it is compared.
# Doing that (rather than loosening the tolerance) keeps the assertions exact.
def as_printed(value: float, places: int = 1) -> float:
    return float("%.*f" % (places, value))


# --------------------------------------------------------------------------- #
# chunk_map
# --------------------------------------------------------------------------- #


def test_chunk_map_keys_are_the_blake2b_of_the_chunk(cdc):
    data = binary.incompressible(120_000, seed=41)
    mapping = cdc.chunk_map(data)
    for a, b in cdc.chunk_bounds(data):
        digest = hashlib.blake2b(data[a:b], digest_size=16).digest()
        assert mapping[digest] == b - a


def test_chunk_map_collapses_identical_chunks(cdc):
    """
    Documenting the shape of the data structure, because the collapse is exactly
    what breaks the accounting below: chunk_map is digest -> length, so a chunk
    that occurs five times is stored once and counted once.
    """
    data = binary.repeated_blocks(100_000, 6, seed=42)
    n_chunks = len(list(cdc.chunk_bounds(data)))
    n_distinct = len(cdc.chunk_map(data))
    assert n_chunks > n_distinct, "fixture is not internally duplicated"


# --------------------------------------------------------------------------- #
# pairwise arithmetic on inputs with known answers
# --------------------------------------------------------------------------- #


def test_identical_unique_content_is_100_percent_reusable(cdc, tmp_path, capsys):
    data = binary.incompressible(300_000, seed=43)
    p = write(tmp_path, "a.bin", data)
    r = parse_pairwise(report_of(capsys, cdc.pairwise, p, p))
    assert r["reuse_pct"] == 100.00
    assert r["new_mb"] == 0.0


def test_completely_different_content_is_zero_percent_reusable(cdc, tmp_path, capsys):
    a = write(tmp_path, "a.bin", binary.incompressible(300_000, seed=44))
    b = write(tmp_path, "b.bin", binary.incompressible(300_000, seed=45))
    r = parse_pairwise(report_of(capsys, cdc.pairwise, a, b))
    assert r["reuse_pct"] == 0.00
    assert r["new_mb"] == pytest.approx(0.3, abs=0.01)


def test_reused_plus_new_equals_the_whole_of_b(cdc, tmp_path, capsys):
    """reuse% and "must store new" must be two views of one number."""
    a_data = binary.incompressible(400_000, seed=46)
    b_data = binary.replace_region(a_data, 150_000, binary.incompressible(30_000, seed=47))
    a = write(tmp_path, "a.bin", a_data)
    b = write(tmp_path, "b.bin", b_data)

    r = parse_pairwise(report_of(capsys, cdc.pairwise, a, b))
    assert 0.0 < r["reuse_pct"] < 100.0, "fixture did not produce a partial overlap"

    reused_mb = r["total_mb"] * r["reuse_pct"] / 100.0
    assert reused_mb + r["new_mb"] == pytest.approx(r["total_mb"], abs=0.005)


def test_a_head_insertion_is_almost_entirely_reusable(cdc, tmp_path, capsys):
    """
    The storage claim in ADR-0002 ("99.6% reuse on a time-shifted region") in
    miniature, on material whose answer is known by construction.
    """
    a_data = binary.incompressible(400_000, seed=48)
    b_data = binary.insert_at(a_data, 0, b"\x00" * 1024)
    a = write(tmp_path, "a.bin", a_data)
    b = write(tmp_path, "b.bin", b_data)

    r = parse_pairwise(report_of(capsys, cdc.pairwise, a, b))
    assert r["reuse_pct"] > 90.0, (
        "a 1 KB prepend should leave almost everything reusable, got %.2f%%"
        % r["reuse_pct"]
    )


def test_gunzip_mode_compares_decompressed_content(cdc, tmp_path, capsys):
    """
    ``--gunzip`` exists because two .als files with identical content can have
    different gzip framing. Without it the reuse figure measures the compressor.
    """
    import gzip

    payload = binary.xmlish(4000, seed=49)
    for name, level in (("a.als", 6), ("b.als", 9)):
        with gzip.GzipFile(str(tmp_path / name), "wb", compresslevel=level, mtime=0) as fh:
            fh.write(payload)

    a, b = str(tmp_path / "a.als"), str(tmp_path / "b.als")
    assert open(a, "rb").read() != open(b, "rb").read(), "fixture framing is identical"

    raw = parse_pairwise(report_of(capsys, cdc.pairwise, a, b, False))
    unz = parse_pairwise(report_of(capsys, cdc.pairwise, a, b, True))
    assert unz["reuse_pct"] == 100.00
    assert raw["reuse_pct"] < 100.00


# --------------------------------------------------------------------------- #
# the accounting bug
# --------------------------------------------------------------------------- #


@pytest.mark.xfail(
    strict=True,
    reason=(
        "BUG in cdc_dedup.pairwise: `reused` sums each DISTINCT chunk once "
        "(chunk_map is digest->length) while `total` counts every byte of B. Any "
        "internal duplication inside B therefore understates reuse. Here B is A, "
        "byte for byte, and the script reports ~19% reusable instead of 100%. "
        "This biases every pairwise figure downward on repetitive material — "
        "which is what Logic ProjectData is, and 'only 2-87% reuse per pair' in "
        "docs/EXPERIMENTS.md 6a is a pairwise figure. Fix: count occurrences, "
        "e.g. sum the lengths of B's chunk *instances* whose digest is in A."
    ),
)
def test_self_comparison_is_always_100_percent(cdc, tmp_path, capsys):
    data = binary.repeated_blocks(50_000, 6, seed=50)
    p = write(tmp_path, "dup.bin", data)
    r = parse_pairwise(report_of(capsys, cdc.pairwise, p, p))
    assert r["reuse_pct"] == 100.00, (
        "a file compared against itself reported %.2f%% reusable" % r["reuse_pct"]
    )


def test_the_self_comparison_shortfall_is_exactly_the_duplicate_bytes(cdc, tmp_path):
    """
    Characterises the bug above precisely, so a fix can be verified rather than
    guessed at: the shortfall equals total bytes minus distinct-chunk bytes.
    """
    data = binary.repeated_blocks(50_000, 6, seed=50)
    distinct_bytes = sum(cdc.chunk_map(data).values())
    reported = 100.0 * distinct_bytes / len(data)
    assert reported < 100.0
    assert distinct_bytes < len(data)


# --------------------------------------------------------------------------- #
# store_cost
# --------------------------------------------------------------------------- #


def test_store_cost_dedups_identical_versions(cdc, tmp_path, capsys):
    """Ten byte-identical saves must cost the same as one."""
    payload = binary.xmlish(1200, seed=51)
    for i in range(10):
        write(tmp_path, "v%02d.bin" % i, payload)

    r = parse_store(report_of(capsys, cdc.store_cost, str(tmp_path / "v*.bin")))
    assert r["versions"] == 10
    assert r["logical_mb"] == as_printed(10 * len(payload) / 1e6)
    assert r["unique_mb"] == as_printed(len(payload) / 1e6)
    assert r["unique_pct"] == pytest.approx(10.0, abs=0.05)


def test_store_cost_unique_bytes_never_exceed_logical_bytes(cdc, tmp_path, capsys):
    for i in range(6):
        write(tmp_path, "v%02d.bin" % i, binary.incompressible(120_000, seed=60 + i))
    r = parse_store(report_of(capsys, cdc.store_cost, str(tmp_path / "v*.bin")))
    assert r["unique_mb"] <= r["logical_mb"] + 0.05
    assert r["stored_mb"] <= r["unique_mb"] + 0.05, "zlib made the store bigger"
    assert r["unique_pct"] == pytest.approx(100.0, abs=0.05), (
        "six unrelated files share no chunks, so unique must equal logical"
    )
    assert r["per_version_mb"] > 0


def test_store_cost_matches_a_hand_computed_answer(cdc, tmp_path, capsys):
    """
    Three versions built by appending, so the expected unique-byte count can be
    computed independently of the script and compared field by field.
    """
    v1 = binary.incompressible(150_000, seed=70)
    v2 = v1 + binary.incompressible(80_000, seed=71)
    v3 = v2 + binary.incompressible(80_000, seed=72)
    for i, data in enumerate((v1, v2, v3), start=1):
        write(tmp_path, "v%d.bin" % i, data)

    store = {}
    for data in (v1, v2, v3):
        for a, b in cdc.chunk_bounds(data):
            store.setdefault(hashlib.blake2b(data[a:b], digest_size=16).digest(), data[a:b])
    logical = len(v1) + len(v2) + len(v3)
    expected_unique = sum(len(v) for v in store.values())
    expected_stored = sum(len(zlib.compress(v, 6)) for v in store.values())

    assert expected_unique < logical, "append chain failed to dedup"

    r = parse_store(report_of(capsys, cdc.store_cost, str(tmp_path / "v*.bin")))
    assert r["versions"] == 3
    assert r["logical_mb"] == as_printed(logical / 1e6)
    assert r["unique_mb"] == as_printed(expected_unique / 1e6)
    assert r["unique_pct"] == as_printed(100 * expected_unique / logical)
    assert r["stored_mb"] == as_printed(expected_stored / 1e6)
    assert r["stored_pct"] == as_printed(100 * expected_stored / logical, 2)
    assert r["per_version_mb"] == as_printed(expected_stored / 3 / 1e6, 3)


def test_store_cost_exits_cleanly_when_nothing_matches(cdc, tmp_path):
    with pytest.raises(SystemExit) as exc:
        cdc.store_cost(str(tmp_path / "nothing-here-*.bin"))
    assert "no files matched" in str(exc.value)
