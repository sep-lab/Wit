#!/usr/bin/env python3
"""
Content-defined chunking (FastCDC-style) dedup harness.

WHAT THIS MEASURES
    How much of file B can be reused from file A when both are stored in a
    content-addressed chunk store. This is the measurement that decides Wit's
    storage design, because it answers the question git cannot: "the producer
    re-rendered this stem — how much do we actually have to store?"

    Content-defined chunking cuts a file at boundaries chosen by a rolling hash
    of the *content*, not at fixed offsets. That makes it immune to insertions
    and deletions shifting everything downstream — which fixed-block hashing
    (and git's whole-blob storage) are not.

KEY RESULT (measured on a real 19.6 MB 24-bit/48k stem)
    trimmed / shifted in time ............ 99.6% reusable
    one 5-second section re-rendered ..... 93.0% reusable
    global EQ change, re-rendered ........  0.00% reusable   <-- the wall

    The last line is why Wit does not version rendered audio. Perceptual
    similarity is not byte similarity: change a gain by 0.5 dB and every sample
    value in the file is a different number.

USAGE
    python3 cdc_dedup.py A.wav B.wav                  # pairwise reuse
    python3 cdc_dedup.py --store 'Backup/*.als'       # whole-chain store cost
    python3 cdc_dedup.py --store 'Backup/*.als' --gunzip   # decompress first

WHAT THIS DOES NOT HANDLE
    - This is a pure-Python gear hash: correct, but slow (~1-2 MB/s). A real
      implementation uses SIMD FastCDC and is 100x faster. Do not read timing
      numbers off this script.
    - Chunk parameters (8 KB average) are not tuned per content type. Audio may
      well prefer larger chunks; that experiment has not been run.
    - No compression is applied in pairwise mode; --store applies zlib so the
      figure is comparable to what a real store would occupy.
"""

from __future__ import annotations

import argparse
import glob
import gzip
import hashlib
import random
import sys
import zlib

# Deterministic gear table. A real implementation ships a fixed published table.
_rng = random.Random(1)
GEAR = [_rng.getrandbits(64) for _ in range(256)]
MASK64 = (1 << 64) - 1


def chunk_bounds(data: bytes, bits: int = 13, min_sz: int = 2048, max_sz: int = 65536):
    """Yield (start, end) content-defined chunk boundaries. ~2^bits average size."""
    mask = (1 << bits) - 1
    h = 0
    start = 0
    i = 0
    n = len(data)
    while i < n:
        h = ((h >> 1) + GEAR[data[i]]) & MASK64
        i += 1
        size = i - start
        if size >= min_sz and ((h & mask) == 0 or size >= max_sz):
            yield start, i
            start = i
            h = 0
    if start < n:
        yield start, n


def chunk_map(data: bytes) -> dict[bytes, int]:
    """digest -> chunk length, for every distinct chunk in data."""
    out: dict[bytes, int] = {}
    for a, b in chunk_bounds(data):
        out[hashlib.blake2b(data[a:b], digest_size=16).digest()] = b - a
    return out


def _read(path: str, gunzip: bool) -> bytes:
    if gunzip:
        with gzip.open(path, "rb") as fh:
            return fh.read()
    with open(path, "rb") as fh:
        return fh.read()


def pairwise(path_a: str, path_b: str, gunzip: bool = False) -> None:
    a = chunk_map(_read(path_a, gunzip))
    data_b = _read(path_b, gunzip)
    b = chunk_map(data_b)
    shared = set(a) & set(b)
    reused = sum(b[k] for k in shared)
    total = len(data_b)
    print(f"  A: {path_a.split('/')[-1]}")
    print(f"  B: {path_b.split('/')[-1]}  ({total/1e6:.2f} MB, {len(b)} chunks)")
    print(f"  reusable from A : {100*reused/total:6.2f}%")
    print(f"  must store new  : {(total-reused)/1e6:6.3f} MB")


def store_cost(pattern: str, gunzip: bool = False) -> None:
    """Total cost of storing an entire version chain in one chunk store."""
    files = sorted(glob.glob(pattern))
    if not files:
        sys.exit(f"no files matched {pattern!r}")
    store: dict[bytes, bytes] = {}
    logical = 0
    for path in files:
        data = _read(path, gunzip)
        logical += len(data)
        for a, b in chunk_bounds(data):
            piece = data[a:b]
            store.setdefault(hashlib.blake2b(piece, digest_size=16).digest(), piece)
    unique = sum(len(v) for v in store.values())
    compressed = sum(len(zlib.compress(v, 6)) for v in store.values())
    print(f"  versions              : {len(files)}")
    print(f"  logical total         : {logical/1e6:8.1f} MB")
    print(f"  unique chunks         : {unique/1e6:8.1f} MB   ({100*unique/logical:.1f}%)")
    print(f"  unique + zlib (stored): {compressed/1e6:8.1f} MB   ({100*compressed/logical:.2f}%)")
    print(f"  average per version   : {compressed/len(files)/1e6:8.3f} MB")


def main() -> None:
    ap = argparse.ArgumentParser(description="Content-defined chunking dedup harness")
    ap.add_argument("a", nargs="?")
    ap.add_argument("b", nargs="?")
    ap.add_argument("--store", help="glob of a version chain; report total store cost")
    ap.add_argument("--gunzip", action="store_true", help="gzip-decompress inputs first (.als)")
    args = ap.parse_args()

    if args.store:
        store_cost(args.store, args.gunzip)
    elif args.a and args.b:
        pairwise(args.a, args.b, args.gunzip)
    else:
        ap.error("provide two files, or --store GLOB")


if __name__ == "__main__":
    main()
