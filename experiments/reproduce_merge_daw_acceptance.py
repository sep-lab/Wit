#!/usr/bin/env python3
"""
Reproduce the EXPERIMENTS.md §5 merge, for issue #1: does Ableton Live accept it?

WHAT THIS DOES
    Everything else in this project is downstream of one unverified assumption: that a
    file Wit produces (merged, or just decompressed-and-recompressed) is a file Ableton
    Live will actually open. EXPERIMENTS.md §5 produced a clean 3-way merge but never
    launched a DAW to check. This script reproduces that merge from real project data on
    your own machine and writes two files for you to open in Live by hand -- the merge
    itself is not the hard part, and never was; nobody has clicked "open" on the result.

WHAT IT PRODUCES
    <out-dir>/merged.als    -- three-way merge of a real edit ("Alice") and a synthetic,
                               disjoint edit ("Bob") against their common ancestor.
    <out-dir>/roundtrip.als -- the ancestor file, gunzipped and re-gzipped, no edits at
                               all. If THIS fails to open, the bug is in repacking, not
                               merging -- see "Why this is a blocker" in issue #1.

USAGE
    python3 experiments/reproduce_merge_daw_acceptance.py \\
        --base   '/path/to/YourProject/Backup/Song [2026-01-01 120000].als' \\
        --alice  '/path/to/YourProject/Backup/Song [2026-01-01 120500].als' \\
        --out-dir /tmp/wit-issue1

    --base and --alice must be two ADJACENT real saves from the same project's Backup/
    chain (base is the earlier one) -- the real edit between them becomes "Alice's" half
    of the merge. "Bob's" half is synthesized by this script: a single volume value on a
    track --alice's real edit does not touch, so the two edits are guaranteed disjoint
    (line-based merge only auto-resolves edits >= 3 lines apart -- EXPERIMENTS.md's own
    caveat -- so disjointness is the whole ballgame, not an incidental detail).

    --bob-track defaults to the first audio track found; pass a specific name (as it
    appears in Ableton) if the default collides with Alice's real edit -- the script
    detects that collision and refuses rather than silently producing a same-line
    "merge" that doesn't test anything.

WHAT TO DO WITH THE OUTPUT
    1. Open merged.als in Ableton Live. Does it open at all?
    2. If it opens: is Alice's real edit present? Is Bob's volume change present (check
       the track --bob-track named, its volume fader)? Any warnings, missing devices, or
       silent data loss?
    3. Open roundtrip.als. Does IT open? (This isolates whether any failure in step 1-2
       comes from merging or from the gunzip/gzip repack alone.)
    4. Report back: exact Ableton Live version, and the answers above.

WHAT THIS DOES NOT HANDLE
    - It does not touch your original project files. Everything is read, decompressed,
      and written under --out-dir only.
    - It does not open Ableton Live -- no DAW automation exists here on purpose, per
      SECURITY.md's scope. Opening the file is a human, one-time act.
    - It does not validate Live's internal semantics, only that the result parses as
      well-formed XML. A file can be valid XML and still be something Live refuses.
"""

from __future__ import annotations

import argparse
import gzip
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

MANUAL_VALUE_RE = re.compile(r'(<Manual Value=")([-\d.]+)(" />)')


def find_track_volume_line(lines: list[str], track_name: str | None) -> tuple[int, str, str]:
    """Return (0-based line index, track name found, current value) of the first
    <Volume><Manual Value="..."/></Volume> under an AudioTrack whose EffectiveName
    matches `track_name` (or the first audio track at all, if `track_name` is None).
    """
    i = 0
    while i < len(lines):
        m = re.search(r'<EffectiveName Value="([^"]*)" />', lines[i])
        if m and (track_name is None or m.group(1) == track_name):
            name = m.group(1)
            # Volume is the first <Volume> block after the track's Name block; the
            # Manual value immediately follows it in every real .als examined so far.
            for j in range(i, min(i + 400, len(lines))):
                if "<Volume>" in lines[j]:
                    for k in range(j, min(j + 5, len(lines))):
                        vm = MANUAL_VALUE_RE.search(lines[k])
                        if vm:
                            return k, name, vm.group(2)
                    break
        i += 1
    raise SystemExit(
        f"could not find a track{' named ' + track_name if track_name else ''} "
        "with a <Volume><Manual> value in --base"
    )


def synthesize_bob(base_lines: list[str], line_idx: int, old_value: str) -> list[str]:
    bob = list(base_lines)
    new_value = f"{(float(old_value) * 0.63 + 0.05) % 1.0:.10f}"
    bob[line_idx] = MANUAL_VALUE_RE.sub(rf"\g<1>{new_value}\g<3>", bob[line_idx])
    return bob, new_value


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    ap.add_argument("--base", required=True, help="earlier real save (the common ancestor)")
    ap.add_argument("--alice", required=True, help="later real save (a real edit)")
    ap.add_argument("--bob-track", default=None, help="track name for Bob's synthetic edit")
    ap.add_argument("--out-dir", required=True, help="scratch directory to write into")
    args = ap.parse_args()

    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)

    base_lines = gzip.open(args.base, "rt", encoding="utf-8").readlines()
    alice_lines = gzip.open(args.alice, "rt", encoding="utf-8").readlines()

    line_idx, track_name, old_value = find_track_volume_line(base_lines, args.bob_track)
    if line_idx < len(alice_lines) and alice_lines[line_idx] != base_lines[line_idx]:
        raise SystemExit(
            f"refusing: Alice's real edit already touches line {line_idx + 1} "
            f"(track {track_name!r}) -- pass a different --bob-track so the two "
            "edits are actually disjoint, per EXPERIMENTS.md §5's own caveat"
        )

    bob_lines, new_value = synthesize_bob(base_lines, line_idx, old_value)
    print(f"Bob's synthetic edit: track {track_name!r}, volume {old_value} -> {new_value}")

    (out / "base.xml").write_text("".join(base_lines), encoding="utf-8")
    (out / "alice.xml").write_text("".join(alice_lines), encoding="utf-8")
    (out / "bob.xml").write_text("".join(bob_lines), encoding="utf-8")
    (out / "merged.xml").write_text("".join(alice_lines), encoding="utf-8")  # merge target

    result = subprocess.run(
        ["git", "merge-file", str(out / "merged.xml"), str(out / "base.xml"), str(out / "bob.xml")],
        capture_output=True,
        text=True,
    )
    print(f"git merge-file exit code: {result.returncode} (0 = clean, >0 = conflict markers left)")
    if result.returncode > 1:
        sys.exit("merge failed outright -- see git merge-file's stderr above")

    merged_xml = (out / "merged.xml").read_text(encoding="utf-8")
    try:
        ET.fromstring(merged_xml)
        print("merged.xml: valid XML")
    except ET.ParseError as e:
        sys.exit(f"merged.xml is NOT valid XML: {e} -- do not open this in Live")

    with gzip.open(out / "merged.als", "wb") as f:
        f.write(merged_xml.encode("utf-8"))

    # Round-trip-only: no edits, isolates repack corruption from merge corruption.
    base_xml = "".join(base_lines)
    with gzip.open(out / "roundtrip.als", "wb") as f:
        f.write(base_xml.encode("utf-8"))
    ET.fromstring(base_xml)  # sanity check on the ancestor itself
    print("roundtrip.als: valid XML, no edits applied")

    for tmp in ("base.xml", "alice.xml", "bob.xml", "merged.xml"):
        (out / tmp).unlink()

    print(f"\nWrote {out / 'merged.als'} and {out / 'roundtrip.als'}.")
    print("Next: open both in Ableton Live and answer issue #1's four questions.")


if __name__ == "__main__":
    main()
