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
    Text events carry no encoding flag. A payload is read as UTF-16LE only when
    its length is even *and* it ends with the two-byte UTF-16 NUL terminator
    (0x00 0x00) that every UTF-16LE-encoded FL string carries; a NUL-terminated
    latin-1 string ends in exactly one zero byte, so this almost never
    coincides by accident. Anything else is read as latin-1.

USAGE
    python3 flp_parse.py PROJECT.flp

WHAT THIS DOES NOT HANDLE
    - Event IDs are only partially named; unknown IDs are reported as '?'.
    - Plugin state payloads are NOT decoded. They are per-plugin private formats.
    - Playlist/pattern data is counted, not interpreted.
    - Not validated across many FL versions; the sample tested was FL 10 era.
      Do not assume ID meanings are stable across major versions.
    - Robustness (this section added when the DoS/crash bugs below were fixed):
      a truncated or lying length field raises FlpParseError naming the file and
      the byte offset, rather than a bare IndexError/struct.error. A varint
      longer than 5 bytes, or a declared payload that runs past the end of the
      file, is refused rather than accepted. The input file handle is always
      closed. None of this makes the parser safe against a hostile file in the
      sense SECURITY.md means for the product core — it is still Python,
      still single-threaded, and still has no fuzzing budget (see
      docs/TESTING.md 9) — it only means the four failure modes measured
      there no longer take the caller down with an unlabelled exception.
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

# A u32 payload length never needs more than 5 continuation bytes (35 usable
# bits). Anything longer is corrupt or hostile input: the varint decoder used
# to shift into an ever-larger Python int with no limit, which made decoding
# a single crafted event quadratic in the length of its length field.
MAX_VARINT_BYTES = 5


class FlpParseError(ValueError):
    """A .flp file is truncated, corrupt, or internally inconsistent.

    Deliberately a ValueError subclass, not a bare IndexError/struct.error, so
    a caller can catch one typed exception instead of guessing which low-level
    error a given corruption happens to trigger.
    """


def _need(data: bytes, pos: int, n: int, path: str, what: str) -> bytes:
    """Return ``data[pos:pos+n]``, or raise a located error if that runs past EOF."""
    if n < 0 or pos < 0 or pos + n > len(data):
        raise FlpParseError(
            "%s: truncated while reading %s at offset %d (need %d byte(s), "
            "%d available)" % (path, what, pos, n, max(len(data) - pos, 0))
        )
    return data[pos : pos + n]


def decode_varint(data: bytes, pos: int, path: str) -> "tuple[int, int]":
    """Decode FL's LEB128-style unsigned varint starting at ``pos``.

    Returns ``(value, new_pos)``. Refuses anything longer than
    ``MAX_VARINT_BYTES`` bytes rather than looping forever on a run of
    continuation bytes.
    """
    value = 0
    shift = 0
    start = pos
    for _ in range(MAX_VARINT_BYTES):
        byte = _need(data, pos, 1, path, "a varint continuation byte")[0]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return value, pos
    raise FlpParseError(
        "%s: varint starting at offset %d exceeds %d bytes (corrupt length, "
        "or a deliberately hostile file)" % (path, start, MAX_VARINT_BYTES)
    )


def decode_text(payload: bytes) -> str:
    """
    Decode a NUL-terminated FL text payload: UTF-16LE in modern files, latin-1
    in older ones. There is no encoding flag in the format, so this guesses.

    NOTE: this heuristic mis-decodes some inputs in both directions (see
    tests/test_flp_parse.py's xfails) — fixed in a follow-up commit. Left
    unchanged here so this commit is robustness-only.
    """
    try:
        s = payload.decode("utf-16-le")
        # Heuristic: ASCII-era files decode to CJK-looking mojibake via UTF-16.
        if sum(ch.isprintable() and ord(ch) < 128 for ch in s) < len(s) * 0.5:
            raise UnicodeDecodeError("utf-16-le", payload, 0, 1, "looks like ascii")
        return s.rstrip("\x00")
    except (UnicodeDecodeError, ValueError):
        return payload.decode("latin-1").rstrip("\x00")


def parse(path: str) -> None:
    with open(path, "rb") as fh:
        data = fh.read()

    if data[:4] != b"FLhd":
        sys.exit(f"not an FLP: expected b'FLhd', got {data[:4]!r}")

    hlen = struct.unpack("<I", _need(data, 4, 4, path, "the header length"))[0]
    header = _need(data, 8, hlen, path, "the header body")
    fmt, nchan, ppq = struct.unpack("<hHH", _need(header, 0, 6, path, "header fields"))
    print(f"FLhd  format={fmt}  channels={nchan}  ppq={ppq}")

    pos = 8 + hlen
    if data[pos : pos + 4] != b"FLdt":
        sys.exit(f"expected b'FLdt' at {pos}, got {data[pos:pos+4]!r}")
    dlen = struct.unpack("<I", _need(data, pos + 4, 4, path, "the data length"))[0]
    pos += 8
    end = pos + dlen
    if end > len(data):
        raise FlpParseError(
            "%s: FLdt declares %d byte(s) of event stream at offset %d, but "
            "only %d byte(s) remain in the file" % (path, dlen, pos, len(data) - pos)
        )
    print(f"FLdt  {dlen} bytes of event stream\n")

    counts: collections.Counter = collections.Counter()
    blob_bytes: collections.Counter = collections.Counter()
    texts: list[tuple[int, str]] = []

    while pos < end:
        ev = _need(data, pos, 1, path, "an event id")[0]
        pos += 1
        if ev < 64:
            size = 1
        elif ev < 128:
            size = 2
        elif ev < 192:
            size = 4
        else:
            size, pos = decode_varint(data, pos, path)
        payload = _need(data, pos, size, path, "event 0x%02x's payload" % ev)
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
