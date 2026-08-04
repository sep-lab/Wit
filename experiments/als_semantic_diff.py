#!/usr/bin/env python3
"""
Semantic diff for Ableton Live sets (.als) — the core Wit primitive, prototyped.

WHAT THIS MEASURES
    Whether a DAW project file can be diffed in terms a musician understands
    ("Stem-mixing volume 0.79 -> 0.52") rather than in terms of XML lines.

    A .als is gzipped XML. A raw text diff of two consecutive autosaves reports
    hundreds of changed lines, but most of them are scroll position, zoom level,
    selection state and re-serialised file references. This script extracts an
    explicit semantic model and diffs that instead.

WHY WHITELIST, NOT BLACKLIST
    The obvious approach is to hash each track's XML subtree while excluding
    "churn" tags. We tried it. Blacklists leak: new churn tags keep appearing and
    the diff stays noisy. Naming the fields we care about is boring and correct.

USAGE
    python3 als_semantic_diff.py OLD.als NEW.als
    python3 als_semantic_diff.py --chain "Backup/*.als"

WHAT THIS DOES NOT HANDLE
    - Device *parameter values* are only partially modelled (device presence and
      order are, individual knob values are not). A save that only tweaks a
      filter cutoff may report "no musical change". This is the single biggest
      gap and the main reason this is a prototype, not a product.
    - MIDI note-level diffs are not produced (note counts only).
    - Automation is compared by lane count, not by breakpoint.
    - Live-version schema drift is not handled; tested against Live 12.x.

Standard library only, by design — see AGENTS.md.
"""

from __future__ import annotations

import argparse
import glob
import gzip
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict

TRACK_TAGS = ("AudioTrack", "MidiTrack", "GroupTrack", "ReturnTrack")


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


def build_model(path: str) -> dict:
    """Extract the musically meaningful state of a Live set."""
    with gzip.open(path, "rb") as fh:
        root = ET.parse(fh).getroot()

    live_set = root.find("LiveSet")
    model = {
        "creator": root.get("Creator"),
        "tempo": None,
        "tracks": {},
        "samples": {},  # sample basename -> set of track names using it
    }

    master = live_set.find("MasterTrack")
    if master is not None:
        tempo = master.find(".//Tempo/Manual")
        if tempo is not None:
            model["tempo"] = _num(tempo.get("Value"))

    for tracks in live_set.findall("Tracks"):
        for tr in tracks:
            if tr.tag not in TRACK_TAGS:
                continue
            chain = tr.find("DeviceChain")
            mixer = chain.find("Mixer") if chain is not None else None

            clips = {}
            if chain is not None:
                for clip in chain.findall(".//ArrangerAutomation/Events/*"):
                    sample = (_val(clip, ".//SampleRef/FileRef/RelativePath") or "")
                    sample = sample.split("/")[-1]
                    clips[clip.get("Id")] = {
                        "name": _val(clip, "Name"),
                        "start": _num(_val(clip, "CurrentStart")),
                        "end": _num(_val(clip, "CurrentEnd")),
                        "sample": sample,
                        "disabled": _val(clip, "Disabled"),
                    }

            name = _val(tr, "Name/EffectiveName") or "?"
            devices = [d.tag for d in chain.findall(".//Devices/*")] if chain is not None else []

            model["tracks"][tr.get("Id")] = {
                "name": name,
                "kind": tr.tag,
                "color": _val(tr, "Color"),
                "volume": _num(_val(mixer, "Volume/Manual")),
                "pan": _num(_val(mixer, "Pan/Manual")),
                "speaker": _val(mixer, "Speaker/Manual"),
                "devices": devices,
                "clips": clips,
                "automation_lanes": len(tr.findall(".//AutomationEnvelope")),
                "notes": len(tr.findall(".//MidiNoteEvent")),
            }
            for c in clips.values():
                if c["sample"]:
                    model["samples"].setdefault(c["sample"], set()).add(name)

    return model


def diff_models(a: dict, b: dict) -> list[str]:
    """Return human-readable changes from model a to model b."""
    out: list[str] = []

    if a["tempo"] != b["tempo"]:
        out.append(f"TEMPO   {a['tempo']} -> {b['tempo']} BPM")

    # --- Coalesce sample renames -------------------------------------------
    # A single rename in the sample folder rewrites every clip that references
    # it. On real material one rename produced 425 clip-level changes. Detect
    # the rename once and suppress the fan-out, or the diff is unreadable.
    renames: dict[tuple[str, str], int] = defaultdict(int)
    for tid in set(a["tracks"]) & set(b["tracks"]):
        ca, cb = a["tracks"][tid]["clips"], b["tracks"][tid]["clips"]
        for cid in set(ca) & set(cb):
            if ca[cid]["sample"] != cb[cid]["sample"] and ca[cid]["sample"] and cb[cid]["sample"]:
                renames[(ca[cid]["sample"], cb[cid]["sample"])] += 1
    for (old, new), count in sorted(renames.items(), key=lambda kv: -kv[1]):
        out.append(f"SAMPLE~ '{old}' -> '{new}'  ({count} clip reference(s))")

    ka, kb = set(a["tracks"]), set(b["tracks"])

    for tid in sorted(kb - ka):
        t = b["tracks"][tid]
        out.append(f"TRACK+  added '{t['name']}' ({t['kind']})")
    for tid in sorted(ka - kb):
        out.append(f"TRACK-  removed '{a['tracks'][tid]['name']}'")

    for tid in sorted(ka & kb):
        ta, tb = a["tracks"][tid], b["tracks"][tid]
        label = tb["name"]

        if ta["name"] != tb["name"]:
            out.append(f"TRACK~  renamed '{ta['name']}' -> '{tb['name']}'")
        for field, human in (
            ("volume", "volume"),
            ("pan", "pan"),
            ("speaker", "output enabled"),
            ("color", "color"),
        ):
            if ta[field] != tb[field]:
                out.append(f"MIX~    [{label}] {human}: {ta[field]} -> {tb[field]}")

        if ta["devices"] != tb["devices"]:
            added = [d for d in tb["devices"] if d not in ta["devices"]]
            removed = [d for d in ta["devices"] if d not in tb["devices"]]
            if added:
                out.append(f"FX+     [{label}] added: {', '.join(added)}")
            if removed:
                out.append(f"FX-     [{label}] removed: {', '.join(removed)}")
            if not added and not removed:
                out.append(f"FX~     [{label}] device chain reordered")

        if ta["automation_lanes"] != tb["automation_lanes"]:
            out.append(
                f"AUTO~   [{label}] automation lanes "
                f"{ta['automation_lanes']} -> {tb['automation_lanes']}"
            )
        if ta["notes"] != tb["notes"]:
            out.append(f"MIDI~   [{label}] note count {ta['notes']} -> {tb['notes']}")

        ca, cb = ta["clips"], tb["clips"]
        for cid in sorted(set(cb) - set(ca)):
            c = cb[cid]
            out.append(f"CLIP+   [{label}] added '{c['name'] or c['sample']}' at bar {c['start']}")
        for cid in sorted(set(ca) - set(cb)):
            c = ca[cid]
            out.append(f"CLIP-   [{label}] removed '{c['name'] or c['sample']}' at bar {c['start']}")
        for cid in sorted(set(ca) & set(cb)):
            x, y = ca[cid], cb[cid]
            nm = y["name"] or y["sample"]
            if (x["start"], x["end"]) != (y["start"], y["end"]):
                out.append(
                    f"CLIP~   [{label}] '{nm}' {x['start']}-{x['end']} -> {y['start']}-{y['end']}"
                )
            if x["disabled"] != y["disabled"]:
                state = "muted" if y["disabled"] == "true" else "unmuted"
                out.append(f"CLIP~   [{label}] '{nm}' {state}")

    return out


def report(old: str, new: str, limit: int = 40) -> int:
    changes = diff_models(build_model(old), build_model(new))
    if not changes:
        print("  no musical change detected (view / bookkeeping only)")
        return 0
    print(f"  {len(changes)} semantic change(s)")
    for line in changes[:limit]:
        print(f"    {line}")
    if len(changes) > limit:
        print(f"    ... and {len(changes) - limit} more")
    return len(changes)


def main() -> None:
    ap = argparse.ArgumentParser(description="Semantic diff for Ableton Live sets")
    ap.add_argument("old", nargs="?", help="older .als")
    ap.add_argument("new", nargs="?", help="newer .als")
    ap.add_argument("--chain", help="glob of .als files to diff pairwise in sorted order")
    ap.add_argument("--limit", type=int, default=40, help="max changes to print per pair")
    args = ap.parse_args()

    if args.chain:
        files = sorted(glob.glob(args.chain))
        if len(files) < 2:
            sys.exit(f"need at least 2 files matching {args.chain!r}, found {len(files)}")
        for older, newer in zip(files, files[1:]):
            print(f"### {older.split('/')[-1]}  ->  {newer.split('/')[-1]}")
            report(older, newer, args.limit)
            print()
        return

    if not (args.old and args.new):
        ap.error("provide OLD and NEW, or --chain GLOB")
    report(args.old, args.new, args.limit)


if __name__ == "__main__":
    main()
