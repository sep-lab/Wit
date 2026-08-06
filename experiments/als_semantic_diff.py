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
    - Robustness (this section added when the DoS/crash bugs below were fixed):
      a document with a DOCTYPE internal subset containing only internal
      entities (the "billion laughs" shape) is refused outright, since Ableton
      never writes a DOCTYPE at all; a document with more than
      MAX_XML_ELEMENTS elements, or more than MAX_XML_BYTES of decompressed
      content, is refused rather than fully materialised. None of this is a
      claim that the parser is hardened against arbitrary hostile input — see
      docs/TESTING.md 9 — only that the four failure modes measured there no
      longer take the caller down silently or unboundedly.

Standard library only, by design — see AGENTS.md.
"""

from __future__ import annotations

import argparse
import glob
import gzip
import io
import os
import re
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict

TRACK_TAGS = ("AudioTrack", "MidiTrack", "GroupTrack", "ReturnTrack")

# Live's autosave filename: "<Project> [YYYY-MM-DD HHMMSS].als"
AUTOSAVE_NAME = re.compile(r" \[\d{4}-\d{2}-\d{2} \d{6}\]\.als$")

# Generous ceilings for hostile-input defence, not product limits. A real Live
# set can legitimately run to hundreds of thousands of XML elements — measured:
# a real project's .als parses to 183,403 elements from 8.9 MB of decompressed
# XML — so an absolute element or byte cap set low enough to catch a crafted
# flat-element bomb would also reject real material. What actually
# distinguishes the bomb is *compression ratio*: that same real file compresses
# ~19x (8.9 MB -> 473 KB), while a maximally repetitive flat-element bomb
# compresses 400x or more (measured on the 100,000-element case this defends
# against). MAX_DECOMPRESSION_RATIO is the primary defence; MAX_XML_BYTES and
# MAX_XML_ELEMENTS are backstops, set far above anything real, for hostile
# input that also pads its compressed size to dodge the ratio check.
MAX_DECOMPRESSION_RATIO = 200
MAX_XML_BYTES = 256 * 1024 * 1024
MAX_XML_ELEMENTS = 2_000_000

# Ableton Live never writes a DOCTYPE. A DOCTYPE with an internal subset whose
# entities are all internal (no SYSTEM/PUBLIC reference) is exactly how an XML
# "billion laughs" bomb is built, and xml.etree.ElementTree expands internal
# entities without a size limit of its own (docs/TESTING.md 6: "assert it, do
# not assume it" — the interpreter's transitive amplification limit is not a
# defence we own). A DOCTYPE that *does* reference an external entity is left
# alone here; expat already refuses to resolve those with its own
# "external entity" error, and that message is worth keeping intact.
_DOCTYPE_INTERNAL_SUBSET_RE = re.compile(rb"<!DOCTYPE\b[^>\[]*\[(.*?)\]", re.DOTALL)
_EXTERNAL_ENTITY_RE = re.compile(rb"<!ENTITY\b[^>]*\b(?:SYSTEM|PUBLIC)\b", re.IGNORECASE)


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


def _reject_entity_bomb(xml_bytes: bytes, path: str) -> None:
    """Refuse a DOCTYPE internal subset that has no external entity in it.

    See the module docstring and the comment above the compiled patterns for
    why this specific shape, and not "any DOCTYPE", is what gets refused here.
    """
    match = _DOCTYPE_INTERNAL_SUBSET_RE.search(xml_bytes)
    if match and not _EXTERNAL_ENTITY_RE.search(match.group(1)):
        raise ET.ParseError(
            "%s: refusing a DOCTYPE internal subset with only internal "
            "entities (possible XML entity-expansion attack); Ableton Live "
            "never writes a DOCTYPE" % path
        )


def _parse_bounded(data: bytes, path: str) -> ET.Element:
    """Parse XML while capping the number of elements materialised.

    ``ET.parse`` holds the entire tree in memory regardless of how much of it
    the whitelist below actually reads, and Python object overhead per element
    is what actually blew up in the measured bug (an 8.8 KB gzipped .als with
    200,000 trivial elements peaked at 66 MB — ~330 bytes of Python object per
    XML element that is a few bytes on disk). The compression-ratio check in
    ``_read_gzip_capped`` is the real defence against that specific shape;
    this is a backstop, set at MAX_XML_ELEMENTS far above the 183,403 elements
    measured in a real project, for a hostile file that pads its compressed
    size enough to dodge the ratio check but still declares an absurd element
    count. ``iterparse`` still builds the same tree, but it lets this function
    count elements as they close and abort before the count runs away.
    """
    count = 0
    context = iter(ET.iterparse(io.BytesIO(data), events=("start", "end")))
    _, root = next(context)
    for event, _elem in context:
        if event != "end":
            continue
        count += 1
        if count > MAX_XML_ELEMENTS:
            raise ValueError(
                "%s: more than %d XML elements; refusing to parse further "
                "(bounds memory on hostile input, see docs/TESTING.md 6)"
                % (path, MAX_XML_ELEMENTS)
            )
    return root


def _read_gzip_capped(
    path: str,
    byte_cap: int = MAX_XML_BYTES,
    ratio_cap: int = MAX_DECOMPRESSION_RATIO,
) -> bytes:
    """
    Decompress ``path``, refusing once the output exceeds an absolute cap or a
    generous multiple of the file's own compressed size on disk, whichever is
    smaller.

    The ratio check is what actually distinguishes a real, large Live set
    from a decompression bomb — see the comment on MAX_DECOMPRESSION_RATIO.
    Reading in fixed-size chunks matters as much as the cap itself: asking
    ``GzipFile.read(n)`` for a single huge ``n`` makes it pre-allocate a
    buffer of that size before it even looks at the data, which would defeat
    the cap for exactly the hostile/garbage input it exists to bound
    (measured: a 268 MB peak from a 184-byte non-gzip file, just from the
    size of the read request).
    """
    try:
        compressed_size = max(os.path.getsize(path), 1)
    except OSError:
        compressed_size = 1
    # A floor keeps a tiny-but-legitimate file from being choked by the ratio
    # alone; the absolute byte_cap is the real ceiling for anything larger.
    limit = min(byte_cap, max(compressed_size * ratio_cap, 1_000_000))

    chunk_size = 1024 * 1024
    total = 0
    pieces = []
    with gzip.open(path, "rb") as fh:
        while True:
            chunk = fh.read(chunk_size)
            if not chunk:
                break
            pieces.append(chunk)
            total += len(chunk)
            if total > limit:
                raise ValueError(
                    "%s: decompressed content exceeds %d bytes (more than "
                    "%dx its %d-byte compressed size); refusing to parse "
                    "(possible decompression bomb)"
                    % (path, limit, ratio_cap, compressed_size)
                )
    return b"".join(pieces)


def build_model(path: str) -> dict:
    """Extract the musically meaningful state of a Live set."""
    data = _read_gzip_capped(path)
    _reject_entity_bomb(data, path)
    root = _parse_bounded(data, path)

    live_set = root.find("LiveSet")
    if live_set is None:
        raise ValueError(
            "%s is not an Ableton Live set (no <LiveSet> element)" % path
        )
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
    #
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

        # Live names autosaves "<Project> [YYYY-MM-DD HHMMSS].als", and a Backup/
        # folder accumulates every project ever saved into that set. Diffing across
        # two different songs produces a technically-correct, entirely meaningless
        # wall of changes. Split by project name rather than silently emitting nonsense.
        #
        # Only files that follow that convention are grouped. Anything else was named
        # deliberately by the user (v1.als, mix_final.als), so respect the order given
        # and treat it as one chain.
        autosaves = [p for p in files if AUTOSAVE_NAME.search(p.split("/")[-1])]
        if len(autosaves) == len(files):
            lineages = {}
            for path in files:
                stem = path.split("/")[-1]
                lineages.setdefault(stem.split(" [")[0], []).append(path)
        else:
            lineages = {"": files}

        if len(lineages) > 1:
            print(f"  note: {len(lineages)} different projects matched this glob:")
            for name, group in sorted(lineages.items()):
                print(f"          {len(group):>3} save(s)  {name}")
            print("        Diffing across two different songs is meaningless, so each")
            print("        project is chained separately below.")
            print()

        for name, group in sorted(lineages.items()):
            if len(group) < 2:
                print(f"### {name}  — only 1 save, nothing to compare\n")
                continue
            if len(lineages) > 1:
                print(f"########## {name} ({len(group)} saves) ##########\n")
            for older, newer in zip(group, group[1:]):
                print(f"### {older.split('/')[-1]}  ->  {newer.split('/')[-1]}")
                report(older, newer, args.limit)
                print()
        return

    if not (args.old and args.new):
        ap.error("provide OLD and NEW, or --chain GLOB")
    report(args.old, args.new, args.limit)


if __name__ == "__main__":
    main()
