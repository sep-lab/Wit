#!/usr/bin/env python3
"""
FL Studio .flp event-stream parser (structural survey).

WHAT THIS MEASURES
    What fraction of a real FL Studio project is structure Wit could model
    versus opaque plugin state it can only store as a blob.

    RESULT on a real project ("Aston Martin Music Remake.flp", FL 10 era):
        1,491 events, 82 distinct event IDs
        ~92% of the file is variable-length binary payload (plugin/channel state)

    That is the plugin wall, quantified. It does not block versioning — Wit
    content-addresses those blobs and reports "the plugin state changed" — but
    it does mean a rich semantic diff of FL projects is much harder than for
    Ableton, where the equivalent state is far smaller and partly readable.

FORMAT (as verified against the local file, consistent with public docs)
    'FLhd' + u32 length + { i16 format, u16 channels, u16 ppq }
    'FLdt' + u32 length + event stream
    Event = u8 id, then a payload sized by the id:
        id   0- 63 : 1 byte
        id  64-127 : 2 bytes
        id 128-191 : 4 bytes
        id 192-255 : varint length, then that many bytes
    Text events are ASCII in older files and UTF-16LE in newer ones; this script
    tries UTF-16LE first and falls back to latin-1.

USAGE
    python3 flp_parse.py PROJECT.flp

WHAT THIS DOES NOT HANDLE
    - Event IDs are only partially named; unknown IDs are reported as '?'.
    - Plugin state payloads are NOT decoded. They are per-plugin private formats.
    - Playlist/pattern data is counted, not interpreted.
    - Not validated across many FL versions; the sample tested was FL 10 era.
      Do not assume ID meanings are stable across major versions.
"""

from __future__ import annotations

import argparse
import collections
import struct
import sys

EVENT_NAMES = {
    0: "ChanEnabled", 9: "LoopActive", 11: "Shuffle", 13: "PatLength", 16: "LoopType",
    17: "ChanType", 28: "LoopOn", 30: "MixerChan", 32: "Swing", 64: "NewChan",
    65: "ChanNew", 66: "PatNew", 96: "PluginColor", 98: "FX", 99: "Fade",
    192: "ChanName", 193: "PatName", 194: "Title", 195: "Comment",
    196: "SampleFileName", 197: "URL", 198: "CommentRTF", 199: "Version",
    200: "RegName", 201: "DefPluginName", 202: "ProjDataPath", 203: "PluginName",
    204: "InsertName", 205: "TimeMarker", 206: "Genre", 207: "Artist",
    208: "PatData", 209: "PluginParams", 210: "ChanParams", 212: "Delay",
    215: "NewPlugin", 216: "PluginIcon", 224: "PluginSettings", 225: "ChanSettings",
    226: "PlayListItems", 227: "AutomationData", 228: "PatCtrlRec",
    229: "InsertRoutes", 233: "RemoteCtrl",
}
TEXT_EVENTS = set(range(192, 208))


def decode_text(payload: bytes) -> str:
    try:
        s = payload.decode("utf-16-le")
        # Heuristic: ASCII-era files decode to CJK-looking mojibake via UTF-16.
        if sum(ch.isprintable() and ord(ch) < 128 for ch in s) < len(s) * 0.5:
            raise UnicodeDecodeError("utf-16-le", payload, 0, 1, "looks like ascii")
        return s.rstrip("\x00")
    except (UnicodeDecodeError, ValueError):
        return payload.decode("latin-1").rstrip("\x00")


def parse(path: str) -> None:
    data = open(path, "rb").read()
    if data[:4] != b"FLhd":
        sys.exit(f"not an FLP: expected b'FLhd', got {data[:4]!r}")

    hlen = struct.unpack("<I", data[4:8])[0]
    fmt, nchan, ppq = struct.unpack("<hHH", data[8 : 8 + hlen])
    print(f"FLhd  format={fmt}  channels={nchan}  ppq={ppq}")

    pos = 8 + hlen
    if data[pos : pos + 4] != b"FLdt":
        sys.exit(f"expected b'FLdt' at {pos}, got {data[pos:pos+4]!r}")
    dlen = struct.unpack("<I", data[pos + 4 : pos + 8])[0]
    pos += 8
    end = pos + dlen
    print(f"FLdt  {dlen} bytes of event stream\n")

    counts: collections.Counter = collections.Counter()
    blob_bytes: collections.Counter = collections.Counter()
    texts: list[tuple[int, str]] = []

    while pos < end:
        ev = data[pos]
        pos += 1
        if ev < 64:
            size = 1
        elif ev < 128:
            size = 2
        elif ev < 192:
            size = 4
        else:
            size = 0
            shift = 0
            while True:
                byte = data[pos]
                pos += 1
                size |= (byte & 0x7F) << shift
                shift += 7
                if not byte & 0x80:
                    break
        payload = data[pos : pos + size]
        pos += size
        counts[ev] += 1
        if ev >= 192:
            blob_bytes[ev] += size
            if ev in TEXT_EVENTS:
                text = decode_text(payload)
                if text.strip():
                    texts.append((ev, text))

    total_blob = sum(blob_bytes.values())
    print(f"{'id':>4} {'count':>6}  {'name':<18} {'payload bytes':>14}")
    for ev, count in counts.most_common(20):
        print(f"{ev:>4} {count:>6}  {EVENT_NAMES.get(ev,'?'):<18} {blob_bytes.get(ev,0):>14}")

    print(f"\nevents: {sum(counts.values())}   distinct ids: {len(counts)}")
    print(
        f"variable-length payload: {total_blob} B "
        f"({100*total_blob/len(data):.1f}% of the {len(data)} B file)"
    )
    print("  ^ this is overwhelmingly opaque plugin/channel state\n")

    print("-- text events --")
    for ev, text in texts[:20]:
        print(f"  [{EVENT_NAMES.get(ev,'?'):<16}] {text[:88]}")

    samples = [t for e, t in texts if e == 196]
    if samples:
        print(f"\nsample references: {len(samples)}")
        if any(s.startswith("%") for s in samples):
            print("  note: FL uses %VARIABLE% path tokens (e.g. %FLStudioData%),")
            print("  which is more portable than the absolute paths Ableton stores.")


def main() -> None:
    ap = argparse.ArgumentParser(description="Survey an FL Studio .flp event stream")
    ap.add_argument("flp")
    parse(ap.parse_args().flp)


if __name__ == "__main__":
    main()
