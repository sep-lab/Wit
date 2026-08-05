"""
Fixture factories for the Wit test suite.

Nothing in this package reads a real DAW project or a real audio file. Every
fixture is constructed in code, because a DAW project file must never be committed
to this repository (see ``.github/workflows/ci.yml`` and AGENTS.md).

    factories.als     — synthetic gzipped Ableton .als
    factories.flp     — synthetic FL Studio .flp byte streams
    factories.binary  — deterministic byte streams for chunking/dedup properties
"""

from __future__ import annotations

__all__ = ["als", "flp", "binary"]
