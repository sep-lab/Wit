#!/usr/bin/env python3
"""
Zero-input demo: see what a Wit diff looks like, without owning a DAW.

WHAT THIS DOES
    Synthesises a small Ableton Live set, applies a sequence of edits a real
    producer would make, writes each version as a genuine gzipped `.als`, and
    runs the real semantic differ (als_semantic_diff.py) over the chain.

    Nothing here is faked for the demo: the files are valid `.als` containers
    and the diff output comes from the same code path that produced every
    number in docs/EXPERIMENTS.md.

WHY IT EXISTS
    The other prototypes need one of your own projects. This one needs nothing,
    so a contributor can see the output ten seconds after cloning and decide
    whether the idea is interesting before hunting for a session to test on.

USAGE
    python3 experiments/demo.py
    python3 experiments/demo.py --keep /tmp/witdemo    # keep the .als files

WHAT THIS DOES NOT HANDLE
    - The generated set is deliberately tiny (6 tracks). A real Live set is
      ~230,000 lines; this is ~200. It exercises the model, not the scale.
    - It writes only the elements the semantic model reads, so these files are
      NOT openable in Ableton Live. They are diff fixtures, not projects.
    - It demonstrates the differ only. Storage, merge and the null test have
      their own scripts.
"""

from __future__ import annotations

import argparse
import copy
import gzip
import os
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))


# --------------------------------------------------------------------------- #
# a minimal Live set, built from plain dicts
# --------------------------------------------------------------------------- #

def new_set():
    """Six tracks, a few clips — the smallest thing that shows every change type."""
    return {
        "tempo": 122.0,
        "tracks": [
            {"id": "100", "name": "Kick", "color": "13", "volume": 0.85, "pan": 0.0,
             "devices": ["Eq8", "Compressor2"], "clips": [
                 {"id": "10", "name": "kick loop", "start": 0.0, "end": 32.0,
                  "sample": "Samples/Imported/kick_old.wav"}]},
            {"id": "101", "name": "Bass", "color": "5", "volume": 0.72, "pan": 0.0,
             "devices": ["Eq8"], "clips": [
                 {"id": "11", "name": "bassline", "start": 0.0, "end": 32.0,
                  "sample": "Samples/Imported/bass.wav"}]},
            {"id": "102", "name": "Corpus metal", "color": "21", "volume": 0.60, "pan": -0.2,
             "devices": ["Corpus"], "clips": [
                 {"id": "12", "name": "Wood hits", "start": 16.0, "end": 24.0,
                  "sample": "Samples/Imported/wood_hits.wav"}]},
            {"id": "103", "name": "Pad", "color": "9", "volume": 0.55, "pan": 0.1,
             "devices": ["Reverb"], "clips": [
                 {"id": "13", "name": "pad swell", "start": 8.0, "end": 40.0,
                  "sample": "Samples/Imported/pad.wav"}]},
            {"id": "104", "name": "Vox", "color": "2", "volume": 0.80, "pan": 0.0,
             "devices": [], "clips": [
                 {"id": "14", "name": "lead vox", "start": 16.0, "end": 48.0,
                  "sample": "Samples/Recorded/vox_take3.wav"}]},
            {"id": "105", "name": "Stem-mixing", "color": "17", "volume": 0.794, "pan": 0.0,
             "devices": ["Limiter"], "clips": []},
        ],
    }


def _v(parent, tag, value):
    el = ET.SubElement(parent, tag)
    el.set("Value", "true" if value is True else "false" if value is False else str(value))
    return el


def to_als_bytes(doc, view_scroll=0):
    """Serialise to a real gzipped .als container."""
    root = ET.Element("Ableton", {
        "MajorVersion": "5", "MinorVersion": "12.0_12300",
        "SchemaChangeCount": "1", "Creator": "Wit demo (synthetic)",
    })
    live_set = ET.SubElement(root, "LiveSet")
    # view state — deliberately present so the differ can prove it ignores it
    _v(live_set, "ScrollerPos", view_scroll)
    tracks_el = ET.SubElement(live_set, "Tracks")

    for t in doc["tracks"]:
        tr = ET.SubElement(tracks_el, "AudioTrack", {"Id": t["id"]})
        name = ET.SubElement(tr, "Name")
        _v(name, "EffectiveName", t["name"])
        _v(name, "UserName", t["name"])
        _v(tr, "Color", t["color"])
        chain = ET.SubElement(tr, "DeviceChain")
        mixer = ET.SubElement(chain, "Mixer")
        _v(ET.SubElement(mixer, "Volume"), "Manual", t["volume"])
        _v(ET.SubElement(mixer, "Pan"), "Manual", t["pan"])
        _v(ET.SubElement(mixer, "Speaker"), "Manual", True)
        devices = ET.SubElement(chain, "Devices")
        for d in t["devices"]:
            ET.SubElement(devices, d)
        main = ET.SubElement(chain, "MainSequencer")
        arr = ET.SubElement(main, "ArrangerAutomation")
        events = ET.SubElement(arr, "Events")
        for c in t["clips"]:
            clip = ET.SubElement(events, "AudioClip", {"Id": c["id"]})
            _v(clip, "Name", c["name"])
            _v(clip, "CurrentStart", c["start"])
            _v(clip, "CurrentEnd", c["end"])
            _v(clip, "Disabled", c.get("disabled", False))
            sref = ET.SubElement(clip, "SampleRef")
            fref = ET.SubElement(sref, "FileRef")
            _v(fref, "RelativePath", c["sample"])

    master = ET.SubElement(live_set, "MasterTrack")
    mchain = ET.SubElement(master, "DeviceChain")
    mixer = ET.SubElement(mchain, "Mixer")
    _v(ET.SubElement(mixer, "Tempo"), "Manual", doc["tempo"])

    xml = b'<?xml version="1.0" encoding="UTF-8"?>\n' + ET.tostring(root, encoding="utf-8")
    return gzip.compress(xml, 6)


# --------------------------------------------------------------------------- #
# the edits — each one a thing a producer actually does
# --------------------------------------------------------------------------- #

def track(doc, name):
    return next(t for t in doc["tracks"] if t["name"] == name)


def edits():
    """(label, mutation) pairs applied in order to build the version chain."""

    def just_scrolled(_doc):
        pass  # nothing musical — the differ must say so

    def turn_down_the_stem_bus(doc):
        track(doc, "Stem-mixing")["volume"] = 0.525

    def trim_the_wood_hits(doc):
        c = track(doc, "Corpus metal")["clips"][0]
        c["start"], c["end"] = 16.0, 24.5

    def add_a_filter_and_mute_the_pad(doc):
        track(doc, "Pad")["devices"].append("AutoFilter")
        track(doc, "Pad")["clips"][0]["disabled"] = True

    def rename_the_kick_sample(doc):
        for t in doc["tracks"]:
            for c in t["clips"]:
                if c["sample"].endswith("kick_old.wav"):
                    c["sample"] = "Samples/Imported/kick_FINAL.wav"

    def new_vocal_take_and_a_harmony_track(doc):
        track(doc, "Vox")["clips"][0]["sample"] = "Samples/Recorded/vox_take7.wav"
        doc["tracks"].append({
            "id": "106", "name": "Vox harmony", "color": "3", "volume": 0.5, "pan": 0.3,
            "devices": ["Reverb"], "clips": [
                {"id": "15", "name": "harmony", "start": 24.0, "end": 48.0,
                 "sample": "Samples/Recorded/vox_harmony.wav"}]})

    def speed_it_up_and_drop_a_track(doc):
        doc["tempo"] = 124.0
        doc["tracks"] = [t for t in doc["tracks"] if t["name"] != "Corpus metal"]

    return [
        ("you scrolled around and hit save", just_scrolled),
        ("turned the stem bus down", turn_down_the_stem_bus),
        ("trimmed the wood hits", trim_the_wood_hits),
        ("added a filter to the pad, muted its clip", add_a_filter_and_mute_the_pad),
        ("renamed the kick sample in Finder", rename_the_kick_sample),
        ("comped a new vocal, added a harmony track", new_vocal_take_and_a_harmony_track),
        ("pushed the tempo, cut a percussion track", speed_it_up_and_drop_a_track),
    ]


def main():
    ap = argparse.ArgumentParser(description="Zero-input demo of the Wit semantic diff")
    ap.add_argument("--keep", metavar="DIR", help="write the .als chain here and keep it")
    args = ap.parse_args()

    out = args.keep or tempfile.mkdtemp(prefix="wit-demo-")
    os.makedirs(out, exist_ok=True)

    doc = new_set()
    versions = [os.path.join(out, "v00.als")]
    with open(versions[0], "wb") as fh:
        fh.write(to_als_bytes(doc))

    labels = []
    for i, (label, mutate) in enumerate(edits(), start=1):
        doc = copy.deepcopy(doc)
        mutate(doc)
        path = os.path.join(out, "v%02d.als" % i)
        # bump view state on every save, exactly as Live does
        with open(path, "wb") as fh:
            fh.write(to_als_bytes(doc, view_scroll=i * 137))
        versions.append(path)
        labels.append(label)

    total = sum(os.path.getsize(p) for p in versions)
    print()
    print("  Synthesised %d versions of a 6-track Live set (%d bytes of .als total)"
          % (len(versions), total))
    print("  in %s" % out)
    print()
    print("  Now running the real semantic differ over the chain.")
    print("  Note the first save: you only scrolled, and it says so.")
    print("=" * 74)

    differ = os.path.join(HERE, "als_semantic_diff.py")
    for i, (a, b) in enumerate(zip(versions, versions[1:])):
        print()
        print("  save %d -> %d   (%s)" % (i, i + 1, labels[i]))
        print("  " + "-" * 70)
        proc = subprocess.run([sys.executable, differ, a, b],
                              capture_output=True, text=True)
        sys.stdout.write(proc.stdout or proc.stderr)

    print()
    print("=" * 74)
    print("  That is the whole idea: a save is 'nothing changed' or a short list of")
    print("  things a musician recognises — not thousands of lines of XML.")
    print()
    print("  Try it on your own work (Live keeps autosaves in <Project>/Backup/):")
    print("    python3 experiments/als_semantic_diff.py --chain '<Project>/Backup/*.als'")
    print()

    if not args.keep:
        shutil.rmtree(out, ignore_errors=True)
    else:
        print("  .als chain kept in %s" % out)
        print()


if __name__ == "__main__":
    main()
