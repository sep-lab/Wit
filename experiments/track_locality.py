#!/usr/bin/env python3
"""
Track locality: how many tracks does a real save actually touch?

WHAT THIS MEASURES
    For each consecutive pair of autosaves in a chain, how many of the
    project's tracks have a track subtree that actually changed. This is the
    number behind the claim "edits are local, so track-granular merge is
    viable" (docs/EXPERIMENTS.md Sec3) -- and, per
    https://github.com/sep-lab/Wit/issues/11, that claim was published without
    a script and without a frozen exclusion set, so the number swung 4x
    (median 3 to median 13) depending on what got excluded. This script fixes
    that by freezing the exclusion set in code, naming every entry, and
    computing BOTH a blacklist and a whitelist number so the gap between them
    is visible instead of hidden in an undocumented parameter.

TWO METHODS, REPORTED SIDE BY SIDE
    BLACKLIST (hash everything, exclude known churn)
        Hash each track's whole XML subtree as a canonical (tag, attrs, text,
        children) tuple, after dropping the elements/attributes in
        TRACK_BLACKLIST_ELEMENTS / TRACK_BLACKLIST_ATTRS below. Two tracks
        hash equal iff every field Ableton writes, other than the excluded
        churn, is identical.

        This is the approach used (undocumented) for the original Sec 3
        number, and AGENTS.md's own standing warning about it is the reason
        this script exists: "Blacklists leak: new churn tags keep appearing
        and the diff stays noisy." Expect this number to be less flattering
        than the whitelist number, honestly reported either way.

    WHITELIST (name the fields that matter) -- the AGENTS.md-preferred approach
        Extract only the fields a musician would call "the track": name,
        color, volume, pan, on/off, output routing, the device chain (by
        type, order), and each clip's sample, position and mute state. Two
        tracks are "the same" iff all of that matches, regardless of what
        else Ableton rewrote around it (LomId, FileRef counters, warp
        analysis, scroll position, ...). This is almost exactly
        `als_semantic_diff.py`'s per-track model, minus the track id itself,
        hashed instead of diffed field-by-field.

    The whitelist is the one Wit's design should trust; the blacklist is
    reported for comparison because the issue this script closes exists
    specifically to make that comparison possible, not to hide it.

WHY THE BLACKLIST SET IS FROZEN, NOT TUNED
    Every entry below has a one-line reason. None of them were chosen to hit
    a target number -- they were chosen by inspecting real diffs between
    saves that `als_semantic_diff.py` independently reports as containing NO
    musical change (see the pairs this script is run against), and recording
    what was different anyway. If a new churn field is discovered later, add
    it here with its own comment and re-run; do not silently retune the
    result to match a previously published figure.

METHODOLOGY NOTE, MATCHING docs/EXPERIMENTS.md Sec 3
    Both counts include added and removed tracks as "changed" for that pair,
    on the theory that a track that did not exist before (or after) trivially
    differs. On every chain measured for this script no track was ever added
    or removed, so this only matters if you point it at a chain that isn't
    the stable-track-count case documented in EXPERIMENTS.md Sec 2.

USAGE
    python3 track_locality.py OLD.als NEW.als
    python3 track_locality.py --chain 'Backup/*.als'

WHAT THIS DOES NOT HANDLE
    - Automation *breakpoint values* are not part of the whitelist (only
      automation lane *count*, matching als_semantic_diff.py's model), so a
      save that only redraws an automation curve counts as unchanged under
      the whitelist even though it musically is not. This mirrors
      als_semantic_diff.py's own documented gap; closing it is the same
      unresolved work item as there.
    - The blacklist set was built by inspecting one real project (see the
      module docstring above) and is not claimed exhaustive -- new Ableton
      versions may write new churn fields. That is precisely the leak
      AGENTS.md warns about, which is the whole reason the whitelist number
      is the one to trust, and why this script prints both instead of one.
    - Whichever the two counts disagree on, per pair, is printed, so a reader
      can see exactly which tracks the two methods score differently on and
      why (usually: a track whose only change is inside the blacklist's own
      exclusion set, e.g. a warp-marker-only re-analysis).
    - Standard library only, no hardening against hostile input (unlike
      als_semantic_diff.py's decompression-bomb defences) -- do not point
      this at untrusted files, see experiments/README.md and SECURITY.md.
"""

from __future__ import annotations

import argparse
import glob
import gzip
import json
import re
import statistics
import sys
import xml.etree.ElementTree as ET
from hashlib import sha256

TRACK_TAGS = ("AudioTrack", "MidiTrack", "GroupTrack", "ReturnTrack")

# Live's autosave filename: "<Project> [YYYY-MM-DD HHMMSS].als" -- identical
# convention to als_semantic_diff.py's AUTOSAVE_NAME, reused rather than
# reinvented so a Backup/ folder mixing several project lineages is grouped
# the same way by every script in this directory.
AUTOSAVE_NAME = re.compile(r" \[\d{4}-\d{2}-\d{2} \d{6}\]\.als$")

# ---------------------------------------------------------------------------
# BLACKLIST -- frozen, named, commented. See the module docstring for why
# this exists and why it is not the recommended approach.
# ---------------------------------------------------------------------------

# Whole elements dropped from the hash -- not just their value, their entire
# subtree, so neither their content nor their presence/count affects the
# hash. Each is either pure UI/selection state, an internal bookkeeping
# pointer that is reassigned without any content change, or output of Live's
# audio-analysis pass that can shift on re-analysis with no user edit.
TRACK_BLACKLIST_ELEMENTS: dict[str, str] = {
    # --- view / selection state: never musical -----------------------------
    "IsContentSelectedInDocument": "UI selection flag for this track in the editor",
    "SelectedEnvelope": "which automation lane is selected in the UI, not its content",
    "TrackUnfolded": "whether the track's automation lanes are expanded in the UI",
    "ScrollerTimePreserver": "container for the arrangement scroller's visible time "
        "range (LeftTime/RightTime) -- observed to change between saves that "
        "als_semantic_diff.py reports as having no musical change at all",
    "TimeSelection": "container for the selected time range (AnchorTime/OtherTime) "
        "in the arranger, i.e. what's highlighted, not what changed",
    "ScrollerPos": "horizontal/vertical scroll position",
    "CurrentZoom": "zoom level in the editor",
    "ClientSize": "editor pane pixel dimensions",
    "HighlightedTrackIndex": "which track row is highlighted in the UI",
    "CurrentTime": "playhead position",
    # --- internal object-graph bookkeeping: reassigned, not content --------
    "LomId": "Live Object Model pointer id; EXPERIMENTS.md Sec3 notes 264/7135 "
        "real LomIds are non-zero and meaningful for STORAGE, but safe to "
        "exclude for a DISPLAY/locality diff -- this script only ever hashes "
        "for locality, never for what Wit stores",
    "LomIdView": "the LomId of the UI view bound to this object, not the object itself",
    "PointeeId": "internal pointer-table bookkeeping id",
    "NextPointeeId": "the document's next-pointer-id counter -- global state, not "
        "per-track content, but harmless to exclude per-track too",
    "Pointee": "a bare internal pointer (its only content is its own Id attribute)",
    # --- audio-analysis output: regenerated by Live, not a manual edit -----
    "WarpMarker": "warp-grid marker; Live's transient analysis can re-place these "
        "on reload with no user edit, and EXPERIMENTS.md notes 5,112 of them "
        "in one real project -- see the issue's own measurement that "
        "excluding this tag alone moves the median from 13 to 11",
    "OnsetEvent": "transient/onset detection output (under UserOnsets), "
        "regenerated by Live's audio analysis, not user-entered",
    # --- file/version bookkeeping, not content ------------------------------
    "LastModDate": "filesystem-style modification timestamp on a sample reference",
    "OverwriteProtectionNumber": "Live's own save-collision counter",
}

# Attributes dropped from specific elements, keeping the element (and its
# real children) in the hash. These are positional counters: EXPERIMENTS.md
# Sec1 measured that FileRef/AuPreset ids shift by +1 whenever ANYTHING
# upstream in the whole file is inserted, so two saves that changed nothing
# on this track can still get a different id here purely because some other
# track's sample list grew by one entry. Same shape of bug for
# AutomationTarget/ModulationTarget ids, which are allocated from the same
# kind of global, order-dependent counter.
TRACK_BLACKLIST_ATTRS: dict[str, set[str]] = {
    "FileRef": {"Id"},
    "AuPreset": {"Id"},
    "AutomationTarget": {"Id"},
    "ModulationTarget": {"Id"},
}


def _canon(el: ET.Element):
    """Canonicalize an element into a hashable tuple, applying the blacklist.

    Returns None for an element that should be dropped entirely (the caller
    filters those out of the parent's child list, so a dropped element
    affects neither content nor count).
    """
    if el.tag in TRACK_BLACKLIST_ELEMENTS:
        return None
    drop_attrs = TRACK_BLACKLIST_ATTRS.get(el.tag, ())
    attrs = tuple(sorted((k, v) for k, v in el.attrib.items() if k not in drop_attrs))
    text = (el.text or "").strip()
    children = tuple(c for c in (_canon(child) for child in el) if c is not None)
    return (el.tag, attrs, text, children)


def blacklist_hash(track: ET.Element) -> str:
    return sha256(repr(_canon(track)).encode("utf-8")).hexdigest()


# ---------------------------------------------------------------------------
# WHITELIST -- name the fields that matter. Deliberately close to
# als_semantic_diff.py's per-track model: if a field is worth diffing there,
# it is worth keying locality on here. The track id itself is excluded from
# the hashed record (it is the join key, not content).
# ---------------------------------------------------------------------------


def _val(node, path, attr="Value", default=None):
    if node is None:
        return default
    el = node.find(path)
    return el.get(attr) if el is not None else default


def _num(s, places=3):
    try:
        return round(float(s), places)
    except (TypeError, ValueError):
        return s


def whitelist_record(track: ET.Element) -> dict:
    chain = track.find("DeviceChain")
    mixer = chain.find("Mixer") if chain is not None else None

    clips = []
    if chain is not None:
        for clip in chain.findall(".//ArrangerAutomation/Events/*"):
            sample = _val(clip, ".//SampleRef/FileRef/RelativePath") or ""
            clips.append(
                {
                    "sample": sample.split("/")[-1],
                    "start": _num(_val(clip, "CurrentStart")),
                    "end": _num(_val(clip, "CurrentEnd")),
                    "disabled": _val(clip, "Disabled"),
                }
            )
    clips.sort(key=lambda c: (c["start"] or 0, c["sample"]))

    devices = [d.tag for d in chain.findall(".//Devices/*")] if chain is not None else []

    return {
        "name": _val(track, "Name/EffectiveName"),
        "kind": track.tag,
        "color": _val(track, "Color"),
        "volume": _num(_val(mixer, "Volume/Manual")),
        "pan": _num(_val(mixer, "Pan/Manual")),
        "on": _val(mixer, "On/Manual"),
        "speaker": _val(mixer, "Speaker/Manual"),
        "devices": devices,
        "clips": clips,
        "automation_lanes": len(track.findall(".//AutomationEnvelope")),
        "notes": len(track.findall(".//MidiNoteEvent")),
    }


def whitelist_hash(track: ET.Element) -> str:
    record = whitelist_record(track)
    return sha256(json.dumps(record, sort_keys=True).encode("utf-8")).hexdigest()


# ---------------------------------------------------------------------------
# Parsing and pairwise comparison
# ---------------------------------------------------------------------------


def load_tracks(path: str) -> dict[str, ET.Element]:
    with gzip.open(path, "rb") as fh:
        data = fh.read()
    root = ET.fromstring(data)
    live_set = root.find("LiveSet")
    if live_set is None:
        raise ValueError("%s is not an Ableton Live set (no <LiveSet> element)" % path)
    tracks = {}
    for group in live_set.findall("Tracks"):
        for tr in group:
            if tr.tag in TRACK_TAGS:
                tracks[tr.get("Id")] = tr
    return tracks


def track_name(tr: ET.Element) -> str:
    return _val(tr, "Name/EffectiveName") or "?"


def compare(old: dict[str, ET.Element], new: dict[str, ET.Element], hash_fn):
    """Return (changed_ids, total_ids) for one pair under one hashing method."""
    old_ids, new_ids = set(old), set(new)
    common = old_ids & new_ids
    changed = {tid for tid in common if hash_fn(old[tid]) != hash_fn(new[tid])}
    changed |= old_ids ^ new_ids  # added/removed tracks trivially count as changed
    total = old_ids | new_ids
    return changed, total


def report_pair(old_path: str, new_path: str) -> tuple[int, int, int]:
    old, new = load_tracks(old_path), load_tracks(new_path)
    bl_changed, total = compare(old, new, blacklist_hash)
    wl_changed, _ = compare(old, new, whitelist_hash)

    names = {tid: track_name(new[tid] if tid in new else old[tid]) for tid in total}
    print(f"### {old_path.split('/')[-1]}  ->  {new_path.split('/')[-1]}")
    print(f"  blacklist: {len(bl_changed)} of {len(total)} tracks changed")
    print(f"  whitelist: {len(wl_changed)} of {len(total)} tracks changed")

    only_bl = bl_changed - wl_changed
    only_wl = wl_changed - bl_changed
    if only_bl:
        print(
            "  blacklist-only (churn the blacklist missed, or a real edit the "
            "whitelist doesn't model -- see WHAT THIS DOES NOT HANDLE):"
        )
        for tid in sorted(only_bl):
            print(f"    - {names[tid]!r} (Id {tid})")
    if only_wl:
        print("  whitelist-only (a field the blacklist doesn't exclude still churned):")
        for tid in sorted(only_wl):
            print(f"    - {names[tid]!r} (Id {tid})")
    print()
    return len(bl_changed), len(wl_changed), len(total)


def summarize(label: str, counts: list[int]) -> None:
    if not counts:
        print(f"{label}: no pairs")
        return
    print(
        f"{label}: median {statistics.median(counts):g}, "
        f"range {min(counts)}-{max(counts)}  (n={len(counts)} pairs: {counts})"
    )


def main() -> None:
    ap = argparse.ArgumentParser(description="Track-locality measurement for Ableton Live sets")
    ap.add_argument("old", nargs="?", help="older .als")
    ap.add_argument("new", nargs="?", help="newer .als")
    ap.add_argument("--chain", help="glob of .als files to compare pairwise in sorted order")
    args = ap.parse_args()

    if args.chain:
        files = sorted(glob.glob(args.chain))
        if len(files) < 2:
            sys.exit(f"need at least 2 files matching {args.chain!r}, found {len(files)}")

        # Same lineage-grouping convention as als_semantic_diff.py --chain.
        autosaves = [p for p in files if AUTOSAVE_NAME.search(p.split("/")[-1])]
        if len(autosaves) == len(files):
            lineages: dict[str, list[str]] = {}
            for path in files:
                stem = path.split("/")[-1]
                lineages.setdefault(stem.split(" [")[0], []).append(path)
        else:
            lineages = {"": files}

        if len(lineages) > 1:
            print(f"  note: {len(lineages)} different projects matched this glob:")
            for name, group in sorted(lineages.items()):
                print(f"          {len(group):>3} save(s)  {name}")
            print("        Each project is chained separately below.\n")

        for name, group in sorted(lineages.items()):
            if len(group) < 2:
                print(f"### {name}  — only 1 save, nothing to compare\n")
                continue
            if len(lineages) > 1:
                print(f"########## {name} ({len(group)} saves) ##########\n")
            bl_counts, wl_counts = [], []
            for older, newer in zip(group, group[1:]):
                bl, wl, _total = report_pair(older, newer)
                bl_counts.append(bl)
                wl_counts.append(wl)
            summarize("BLACKLIST changed-tracks per save", bl_counts)
            summarize("WHITELIST changed-tracks per save", wl_counts)
            print()
        return

    if not (args.old and args.new):
        ap.error("provide OLD and NEW, or --chain GLOB")
    report_pair(args.old, args.new)


if __name__ == "__main__":
    main()
