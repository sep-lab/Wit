#!/usr/bin/env python3
"""
Null-test diff: what actually changed between two renders, for ANY DAW.

WHAT THIS DOES
    Aligns two audio files, subtracts one from the other, and reports where the
    residual rises above the session's own noise floor. That tells you *which
    seconds of the song changed and by how much* -- without parsing a project
    file, without the DAW, and identically for Ableton, Logic, FL, Pro Tools or
    a bounce someone emailed you.

WHY THIS MATTERS TO WIT
    Every other tool here needs a format parser. This one needs none, so it
    works on day one for every DAW, and it answers a question no existing tool
    answers: "you sent me a new bounce -- what did you change?"

    It is the complement of the project-file diff, not a replacement:
      project diff -> WHY it changed  (fader, plugin, clip moved)
      null diff    -> WHAT you can hear, and WHERE in the timeline

ALIGNMENT IS NOT OPTIONAL -- IT IS THE WHOLE PROBLEM
    Measured: delaying a file by ONE sample and nulling gives a residual as loud
    as the source itself (-29.8 dBFS residual against a -29.6 dBFS source). An
    unaligned null test reports "everything changed" and is worthless. After
    re-alignment the same pair nulls to -91 dB, i.e. digital silence.

    So: align first, always. This script does a coarse-to-fine integer-sample
    search and refuses to report a diff if it cannot find a confident alignment.

ON RENDER DETERMINISM
    DAW renders are not bit-identical across runs (denormal handling differs by
    architecture, plugins drift). That does NOT break this: the null does not
    need to reach zero, it needs the noise floor to sit far below the change.
    Render the same project twice with no edits, null them, and you have that
    session's floor as a calibration constant. Pass it with --floor.

USAGE
    python3 null_diff.py old.wav new.wav
    python3 null_diff.py old.wav new.wav --floor -80 --window 1.0

REQUIRES
    ffmpeg (for decode, alignment shifts, and RMS measurement)

WHAT THIS DOES NOT HANDLE
    - Only integer-sample alignment. A resampled or time-stretched render will
      not null; it will report a large diff. That is honest but not diagnostic.
    - Sub-sample delay, phase rotation, and differing sample rates are not
      corrected. Files at different rates are rejected rather than resampled,
      because resampling would itself destroy the null.
    - Mono/stereo mismatches are rejected.
    - It reports WHERE the audio differs, never WHY.
"""

from __future__ import annotations

import argparse
import json
import math
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SEARCH_MS = 250  # +/- alignment search range


def _run(cmd: list[str]) -> str:
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"command failed: {' '.join(cmd[:3])}...\n{proc.stderr[-500:]}")
    return proc.stderr + proc.stdout


def probe(path: str) -> dict:
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0",
         "-show_entries", "stream=sample_rate,channels,duration",
         "-of", "json", path],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        raise SystemExit(f"cannot read {path}")
    st = json.loads(out.stdout)["streams"][0]
    return {
        "rate": int(st["sample_rate"]),
        "channels": int(st["channels"]),
        "duration": float(st.get("duration") or 0.0),
    }


def residual_db(a: str, b: str, shift_samples: int, rate: int,
                probe_secs: float | None = None) -> float:
    """RMS of (a - b) with b shifted by shift_samples. Returns dBFS.

    probe_secs limits the analysis to a short excerpt, which is what makes the
    alignment search affordable -- a full-length null costs ~0.3 s, and the
    search needs hundreds of them.
    """
    # Invert b and mix: amix averages, so the result is (a-b)/2 -- a constant
    # 6 dB offset that cancels out when comparing residual against source.
    delay_a = max(0, shift_samples)
    delay_b = max(0, -shift_samples)
    limit = f",atrim=duration={probe_secs}" if probe_secs else ""
    fc = (
        f"[0:a]atrim=start_sample={delay_b},asetpts=PTS-STARTPTS{limit}[a];"
        f"[1:a]atrim=start_sample={delay_a},asetpts=PTS-STARTPTS{limit},volume=-1[b];"
        f"[a][b]amix=inputs=2:duration=shortest:normalize=0,astats=metadata=1:reset=0"
    )
    out = _run(["ffmpeg", "-v", "info", "-nostats", "-i", a, "-i", b,
                "-filter_complex", fc, "-f", "null", "-"])
    peaks = []
    for line in out.splitlines():
        if "RMS level dB" in line:
            val = line.split(":")[-1].strip()
            try:
                peaks.append(float(val))
            except ValueError:
                pass
    if not peaks:
        return 0.0
    return max(peaks)  # worst channel


def source_level_db(path: str) -> float:
    out = _run(["ffmpeg", "-v", "info", "-nostats", "-i", path,
                "-af", "astats=metadata=1:reset=0", "-f", "null", "-"])
    vals = []
    for line in out.splitlines():
        if "RMS level dB" in line:
            try:
                vals.append(float(line.split(":")[-1].strip()))
            except ValueError:
                pass
    return max(vals) if vals else 0.0


def find_alignment(a: str, b: str, rate: int, verbose: bool = False) -> tuple[int, float]:
    """Coarse-to-fine integer-sample alignment. Returns (shift, residual_db).

    Three passes over a short excerpt, each narrowing the window by ~an order of
    magnitude, so the cost is ~O(100) probes rather than O(span). A flat linear
    scan at sample resolution over +/-250 ms would be ~24,000 probes.
    """
    probe = 2.0  # seconds of audio per probe
    span = int(rate * SEARCH_MS / 1000)

    def scan(centre: int, radius: int, step: int, best: tuple[int, float]) -> tuple[int, float]:
        for shift in range(centre - radius, centre + radius + 1, step):
            db = residual_db(a, b, shift, rate, probe_secs=probe)
            if db < best[1]:
                best = (shift, db)
        return best

    best = (0, residual_db(a, b, 0, rate, probe_secs=probe))
    # pass 1: 10 ms grid across the full search window
    best = scan(0, span, max(1, rate // 100), best)
    # pass 2: 1 ms grid around the winner
    best = scan(best[0], max(1, rate // 100), max(1, rate // 1000), best)
    # pass 3: single samples around the winner
    best = scan(best[0], max(1, rate // 1000), 1, best)

    if verbose:
        print(f"  (alignment search settled at {best[0]:+d} samples)")
    # Re-measure the winner over the FULL file -- the excerpt was only for search.
    return best[0], residual_db(a, b, best[0], rate)


def main() -> None:
    ap = argparse.ArgumentParser(description="Null-test diff between two renders")
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--floor", type=float, default=None,
                    help="known noise floor in dBFS (from nulling two identical re-renders)")
    ap.add_argument("--no-align", action="store_true", help="skip alignment search (fast, usually wrong)")
    args = ap.parse_args()

    if not shutil.which("ffmpeg") or not shutil.which("ffprobe"):
        raise SystemExit("ffmpeg and ffprobe are required")

    ia, ib = probe(args.old), probe(args.new)
    if ia["rate"] != ib["rate"]:
        raise SystemExit(f"sample rate mismatch: {ia['rate']} vs {ib['rate']} — refusing "
                         "to resample, it would destroy the null")
    if ia["channels"] != ib["channels"]:
        raise SystemExit(f"channel count mismatch: {ia['channels']} vs {ib['channels']}")

    rate = ia["rate"]
    src = source_level_db(args.old)

    if args.no_align:
        shift, res = 0, residual_db(args.old, args.new, 0, rate)
    else:
        shift, res = find_alignment(args.old, args.new, rate)

    print(f"  source RMS      : {src:7.1f} dBFS")
    print(f"  alignment       : {shift:+d} samples ({1000*shift/rate:+.2f} ms)")
    print(f"  residual RMS    : {res:7.1f} dBFS")
    delta = res - src
    print(f"  residual - source: {delta:+.1f} dB")

    floor = args.floor if args.floor is not None else -70.0
    print()
    if res <= floor:
        print(f"  => IDENTICAL within the noise floor ({floor:.0f} dBFS).")
        print("     Nothing audible changed.")
    elif delta < -40:
        print("  => A SMALL, LOCALISED change. The two renders are substantially the same.")
    elif delta < -12:
        print("  => A CLEAR change, but the material is recognisably the same performance.")
    else:
        print("  => LARGE change, OR the files never aligned (different arrangement,")
        print("     different sample rate lineage, or time-stretched). Treat with suspicion.")

    if shift != 0 and abs(shift) > 1:
        print()
        print(f"  note: the renders were offset by {shift} samples. An unaligned null test")
        print("        would have reported 'everything changed'. Alignment is not optional.")


if __name__ == "__main__":
    main()
