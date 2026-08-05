"""
Content-defined chunking: the boundary properties the storage design rests on.

ADR-0002 keeps CDC for audio ("insert/shift tolerance ... 99.6% reuse on a
time-shifted region"). That claim is only true if chunk boundaries are chosen by
content, so that inserting bytes near the start of a file does not renumber every
chunk after it. Experiment 7's 99.59% figure for "prepend 250 ms" is a direct
consequence. If this property breaks, that number silently becomes fiction, and
nothing else in the suite would notice.

These tests assert the property itself, not a ratio.
"""

from __future__ import annotations

import hashlib
import subprocess
import sys

import pytest
from factories import binary

MIN_SZ = 2048
MAX_SZ = 65536


def hashes(cdc, data):
    return [
        hashlib.blake2b(data[a:b], digest_size=16).digest() for a, b in cdc.chunk_bounds(data)
    ]


def sizes(cdc, data):
    return [b - a for a, b in cdc.chunk_bounds(data)]


# --------------------------------------------------------------------------- #
# the core property
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("prefix_len", [1, 7, 100, 2047, 4096, 9001])
def test_inserting_at_the_head_does_not_shift_downstream_chunks(cdc, prefix_len):
    """
    THE property. Insert bytes at offset 0 and every chunk after the first must
    survive byte-identically — that is what "content-defined" means, and it is
    the difference between 99.6% reuse and 0% on a time-shifted region.
    """
    base = binary.incompressible(300_000, seed=11)
    shifted = binary.insert_at(base, 0, b"\xa5" * prefix_len)

    base_h = hashes(cdc, base)
    shifted_h = hashes(cdc, shifted)

    downstream = base_h[1:]
    missing = [i for i, h in enumerate(downstream, start=1) if h not in set(shifted_h)]
    assert not missing, (
        "chunks %s of the original did not survive a %d-byte head insertion; "
        "chunking is behaving like fixed-size blocking" % (missing, prefix_len)
    )

    # stronger: they survive *in order*, contiguously, as the tail of the new file
    tail = shifted_h[-len(downstream):]
    assert tail == downstream


def test_insertion_in_the_middle_only_disturbs_the_local_chunk(cdc):
    base = binary.incompressible(300_000, seed=12)
    edited = binary.insert_at(base, 150_000, b"\x5a" * 1000)

    base_h = set(hashes(cdc, base))
    edited_h = set(hashes(cdc, edited))
    survived = len(base_h & edited_h)

    assert survived >= len(base_h) - 2, (
        "a 1 KB insertion invalidated %d of %d chunks; a content-defined chunker "
        "should lose at most the chunk containing the edit"
        % (len(base_h) - survived, len(base_h))
    )


def test_deletion_resynchronises(cdc):
    base = binary.incompressible(300_000, seed=13)
    edited = binary.delete_region(base, 100_000, 5000)

    base_h = set(hashes(cdc, base))
    edited_h = set(hashes(cdc, edited))
    assert len(base_h & edited_h) >= len(base_h) - 2


def test_appending_leaves_every_earlier_chunk_untouched(cdc):
    base = binary.incompressible(200_000, seed=14)
    grown = base + binary.incompressible(50_000, seed=15)

    base_h = hashes(cdc, base)
    grown_h = hashes(cdc, grown)
    # every chunk except the last (which was still open at EOF) must be reused
    assert base_h[:-1] == grown_h[: len(base_h) - 1]


# --------------------------------------------------------------------------- #
# size bounds
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("seed", [1, 2, 3])
def test_no_chunk_exceeds_the_maximum(cdc, seed):
    data = binary.incompressible(400_000, seed=seed)
    assert max(sizes(cdc, data)) <= MAX_SZ


@pytest.mark.parametrize("seed", [1, 2, 3])
def test_only_the_final_chunk_may_be_below_the_minimum(cdc, seed):
    data = binary.incompressible(400_000, seed=seed)
    s = sizes(cdc, data)
    assert all(x >= MIN_SZ for x in s[:-1]), "an interior chunk fell below min_sz"
    assert s[-1] >= 1


def test_pathological_content_that_never_triggers_a_boundary_is_capped(cdc):
    """
    All-zero input drives the gear hash to a fixed point. Without the max_sz
    guard this produces one chunk the size of the whole file, which would make
    the store useless on silence — and DAW material contains a lot of silence.
    """
    data = b"\x00" * 200_000
    s = sizes(cdc, data)
    assert max(s) == MAX_SZ
    assert sum(s) == len(data)
    assert len(s) == 4  # 3 full chunks + remainder


def test_chunks_tile_the_input_exactly(cdc):
    """No gaps, no overlaps, no lost tail — otherwise reuse arithmetic is nonsense."""
    for seed in (5, 6):
        data = binary.incompressible(250_000, seed=seed)
        bounds = list(cdc.chunk_bounds(data))
        assert bounds[0][0] == 0
        assert bounds[-1][1] == len(data)
        for (_, prev_end), (next_start, _) in zip(bounds, bounds[1:]):
            assert prev_end == next_start
        assert b"".join(data[a:b] for a, b in bounds) == data


@pytest.mark.parametrize("size", [0, 1, 2047, 2048, 2049, 65535, 65536, 65537])
def test_short_and_boundary_length_inputs(cdc, size):
    data = binary.incompressible(size, seed=9)
    bounds = list(cdc.chunk_bounds(data))
    assert sum(b - a for a, b in bounds) == size
    if size == 0:
        assert bounds == []
    else:
        assert bounds[0][0] == 0 and bounds[-1][1] == size


def test_average_chunk_size_is_near_the_configured_target(cdc):
    """
    bits=13 asks for a ~8 KB average. This is a sanity check on the mask, not a
    published number: if someone changes the gear table or the shift and the
    average moves to 800 bytes or 60 KB, every storage estimate in
    docs/EXPERIMENTS.md silently changes meaning.
    """
    data = binary.incompressible(1_000_000, seed=21)
    s = sizes(cdc, data)
    average = sum(s) / len(s)
    assert 4096 <= average <= 24576, "average chunk size drifted to %.0f B" % average


# --------------------------------------------------------------------------- #
# determinism
# --------------------------------------------------------------------------- #


def test_chunking_is_deterministic_within_a_process(cdc):
    data = binary.incompressible(150_000, seed=31)
    assert list(cdc.chunk_bounds(data)) == list(cdc.chunk_bounds(data))


def test_chunking_is_deterministic_across_processes(cdc, experiments_dir, tmp_path):
    """
    A content-addressed store is only content-addressed if two machines cut the
    same file the same way. The gear table is seeded with random.Random(1); this
    catches anyone replacing it with an unseeded or hash-randomised source.
    """
    data = binary.incompressible(150_000, seed=32)
    path = tmp_path / "sample.bin"
    path.write_bytes(data)

    script = (
        "import sys, hashlib;"
        "sys.path.insert(0, %r);"
        "import cdc_dedup as c;"
        "d=open(%r,'rb').read();"
        "print(hashlib.blake2b(repr(list(c.chunk_bounds(d))).encode()).hexdigest())"
        % (str(experiments_dir), str(path))
    )
    runs = set()
    for seed_hash in ("0", "12345"):
        out = subprocess.run(
            [sys.executable, "-c", script],
            capture_output=True,
            text=True,
            check=True,
            env={"PYTHONHASHSEED": seed_hash, "PATH": "/usr/bin:/bin"},
        )
        runs.add(out.stdout.strip())

    in_process = hashlib.blake2b(repr(list(cdc.chunk_bounds(data))).encode()).hexdigest()
    assert len(runs) == 1, "chunking differs between processes"
    assert runs == {in_process}


def test_gear_table_is_the_published_fixed_table(cdc):
    """
    The gear table is part of the on-disk format: change it and every stored
    chunk id changes. Pin it so that is never an accident.
    """
    assert len(cdc.GEAR) == 256
    assert len(set(cdc.GEAR)) == 256, "duplicate gear entries weaken boundary spread"
    assert all(0 <= g < 2**64 for g in cdc.GEAR)
    digest = hashlib.blake2b(
        b"".join(g.to_bytes(8, "little") for g in cdc.GEAR), digest_size=16
    ).hexdigest()
    assert digest == "95b127d796ad492698980bbdc5bdd3fd", (
        "the gear table changed. Every chunk id in every existing store is now "
        "different. If this is intentional it is a format break, not a refactor."
    )
