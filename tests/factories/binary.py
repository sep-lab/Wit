"""
Deterministic byte-stream factories for the chunking / dedup tests.

Everything here is seeded, so a failing CDC test reproduces exactly. Nothing here
is audio: AGENTS.md is explicit that benchmarking on synthetic material and
presenting it as representative of music is not allowed. These streams exist to
test *algorithm properties* (boundary stability, size bounds, accounting), never to
produce a compression or dedup ratio anyone would quote.
"""

from __future__ import annotations

import random
from typing import List

__all__ = [
    "incompressible",
    "repeated_blocks",
    "xmlish",
    "insert_at",
    "replace_region",
    "delete_region",
]


def incompressible(size: int, seed: int = 1) -> bytes:
    """``size`` bytes of seeded pseudo-random data with no internal repetition."""
    return random.Random(seed).randbytes(size)


def repeated_blocks(block_size: int, times: int, seed: int = 1) -> bytes:
    """One random block repeated ``times`` — a stream with known internal duplication."""
    return random.Random(seed).randbytes(block_size) * times


def xmlish(records: int, seed: int = 1, width: int = 60) -> bytes:
    """
    Text that behaves structurally like a serialised DAW project: many short,
    highly similar records. Used where the *shape* of the data matters (long
    verbatim runs between small scattered edits) rather than its entropy.
    """
    rng = random.Random(seed)
    lines: List[str] = ['<?xml version="1.0" encoding="UTF-8"?>', "<Project>"]
    for i in range(records):
        lines.append(
            '  <Event Id="%d" Time="%.6f" Value="%s" />'
            % (i, i * 0.25, "".join(rng.choice("abcdef0123456789") for _ in range(width)))
        )
    lines.append("</Project>")
    return ("\n".join(lines) + "\n").encode("utf-8")


def insert_at(data: bytes, offset: int, payload: bytes) -> bytes:
    """Insert ``payload`` at ``offset``, shifting everything after it."""
    return data[:offset] + payload + data[offset:]


def replace_region(data: bytes, offset: int, payload: bytes) -> bytes:
    """Overwrite ``len(payload)`` bytes at ``offset``; total length unchanged."""
    return data[:offset] + payload + data[offset + len(payload):]


def delete_region(data: bytes, offset: int, length: int) -> bytes:
    """Remove ``length`` bytes at ``offset``, shifting everything after it."""
    return data[:offset] + data[offset + length:]
