#!/usr/bin/env python3
"""
Performance regression gate for the chunking prototype.

WHAT THIS IS FOR — AND WHAT IT IS NOT
    experiments/cdc_dedup.py is a pure-Python gear hash. It runs at single-digit
    MB/s and its own docstring says so: "Do not read timing numbers off this
    script." The production implementation will be Rust (ADR-0004) and roughly
    two orders of magnitude faster.

    So this gate is NOT here to make the prototype fast. It is here to catch an
    accidental algorithmic blow-up — someone rewrites the rolling hash and makes
    it quadratic, or adds a per-byte allocation, and nothing else in CI notices
    because the *answers* are still correct. Correctness tests do not catch that.

    Consequently the failure threshold is deliberately loose. A hosted runner's
    throughput varies by tens of percent between runs; a tight threshold would
    fail honest PRs and get switched off. We fail only on a change big enough to
    be structural.

USAGE
    python3 bench_cdc.py [--baseline FILE] [--mb N] [--repeat N] [--update]

    --update rewrites the baseline from this run. Only do that deliberately, in
    a PR that says why the number moved.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import random
import statistics
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
sys.path.insert(0, os.path.join(REPO, "experiments"))

# Imported after the sys.path line above: the prototypes are not a package, by
# design (experiments/README.md — no install step).
import cdc_dedup


def make_buffer(mb: float) -> bytes:
    """Deterministic buffer, so run-to-run variation is the machine, not the data."""
    rnd = random.Random(20260805)
    return bytes(rnd.getrandbits(8) for _ in range(int(mb * 1_000_000)))


def measure(buf: bytes, repeat: int) -> dict:
    times = []
    chunks = 0
    for _ in range(repeat):
        started = time.perf_counter()
        chunks = sum(1 for _ in cdc_dedup.chunk_bounds(buf))
        times.append(time.perf_counter() - started)
    mb = len(buf) / 1_000_000
    return {
        "chunk_throughput_mb_s": round(mb / min(times), 3),   # best-of, least noisy
        "median_throughput_mb_s": round(mb / statistics.median(times), 3),
        "chunks": chunks,
        "mean_chunk_bytes": round(len(buf) / chunks) if chunks else 0,
        "buffer_mb": round(mb, 2),
        "repeat": repeat,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="CDC throughput regression gate")
    ap.add_argument("--baseline", default=os.path.join(HERE, "bench-baseline.json"))
    ap.add_argument("--mb", type=float, default=4.0)
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--update", action="store_true", help="rewrite the baseline from this run")
    args = ap.parse_args()

    with open(args.baseline, encoding="utf-8") as fh:
        baseline = json.load(fh)

    buf = make_buffer(args.mb)
    result = measure(buf, args.repeat)
    result["python"] = platform.python_version()
    result["platform"] = f"{platform.system()} {platform.machine()}"

    ref = float(baseline["chunk_throughput_mb_s"])
    tolerance = float(baseline.get("regression_tolerance", 0.5))
    floor = float(baseline.get("absolute_floor_mb_s", 0.0))
    provisional = bool(baseline.get("baseline_is_provisional", False))
    got = result["chunk_throughput_mb_s"]
    ratio = got / ref if ref else 0.0

    # A baseline taken on someone's laptop says nothing about a hosted runner, and
    # gating on it would fail honest PRs. Until it has been re-measured in CI, only
    # the absolute floor is enforced. Pretending otherwise would be exactly the kind
    # of unearned number AGENTS.md forbids.
    gate = floor if provisional else max(ref * tolerance, floor)
    ok = got >= gate

    # A structural invariant that a timing number alone would not catch: the
    # chunker must still produce ~8 KB average chunks. A "faster" version that
    # stopped cutting properly would otherwise look like an improvement.
    size_ok = 4096 <= result["mean_chunk_bytes"] <= 16384

    lines = [
        "## Benchmark — pure-Python CDC throughput",
        "",
        "This gate exists to catch **algorithmic** regressions in the prototype chunker,",
        "not to make it fast. It is pure Python at single-digit MB/s by design; production",
        "is Rust (ADR-0004). The threshold is loose on purpose — hosted runners are noisy,",
        "and a gate that fails honest PRs gets ignored.",
        "",
        "| Metric | Value |",
        "|---|---|",
        f"| throughput (best of {result['repeat']}) | **{got} MB/s** |",
        f"| throughput (median) | {result['median_throughput_mb_s']} MB/s |",
        f"| baseline | {ref} MB/s ({baseline.get('measured_on', 'unknown runner')}) |",
        f"| ratio to baseline | {ratio:.2f}x |",
        f"| fail below | {gate:.2f} MB/s "
        + ("(absolute floor only — baseline is provisional) |" if provisional
           else f"(baseline x {tolerance}, floor {floor}) |"),
        f"| mean chunk size | {result['mean_chunk_bytes']} B (expected ~8192 B) |",
        f"| this runner | {result['platform']}, Python {result['python']} |",
        "",
        ("✅ within tolerance" if ok else "❌ **large throughput regression**"),
        ("" if size_ok else "❌ **mean chunk size is out of range — the chunker's cut points changed**"),
        "",
    ]
    if provisional:
        lines += [
            "> ⚠️ **The committed baseline is provisional** — it was taken on a developer",
            "> laptop, not on this runner, so only the absolute floor is being enforced.",
            "> Once these runs look stable, re-baseline from a green CI run:",
            "> `python3 .github/workflows/scripts/bench_cdc.py --update` and clear",
            "> `baseline_is_provisional` in `bench-baseline.json`.",
            "",
        ]
    if ratio > 1.5 and not provisional:
        lines += [
            f"> Throughput is {ratio:.1f}x the baseline. If that is a real improvement,",
            "> refresh it with `bench_cdc.py --update` in a PR that explains the change,",
            "> so the gate keeps its teeth.",
            "",
        ]
    if not ok:
        lines += [
            "> Throughput fell well below the baseline. That is usually a structural change",
            "> in `chunk_bounds` — a per-byte allocation, an accidental O(n²), a copy inside",
            "> the loop. Timing noise does not move a number this far.",
            "",
        ]

    text = "\n".join(lines)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(text + "\n")
    print(text)
    print(json.dumps(result, indent=2))

    if args.update:
        baseline.update(
            {
                "chunk_throughput_mb_s": got,
                "mean_chunk_bytes": result["mean_chunk_bytes"],
                "measured_on": f"{result['platform']}, Python {result['python']}",
                "buffer_mb": result["buffer_mb"],
                "baseline_is_provisional": False,
            }
        )
        with open(args.baseline, "w", encoding="utf-8") as fh:
            json.dump(baseline, fh, indent=2)
            fh.write("\n")
        print(f"baseline updated: {args.baseline}")
        return 0

    return 0 if (ok and size_ok) else 1


if __name__ == "__main__":
    sys.exit(main())
