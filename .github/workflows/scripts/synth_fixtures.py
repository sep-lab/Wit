#!/usr/bin/env python3
"""
Synthetic fixture generator for CI.

WHY THIS EXISTS
    Wit's guardrails forbid committing audio or DAW project files (see AGENTS.md
    and .gitignore), and the real material the published numbers were measured on
    lives on one person's machine. So CI cannot use it. Everything CI checks must
    therefore be *generated at run time*, into a temp directory outside the repo.

WHAT THESE FIXTURES ARE AND ARE NOT
    They are structurally faithful enough to exercise the prototypes: a .als is
    real gzipped XML with the elements the extractor reads, a .flp is a real
    FLhd/FLdt event stream, a save chain has the shape DAW saves actually have
    (long verbatim runs, small scattered edits).

    They are NOT representative material. Nothing measured on them may be quoted
    as a Wit result. Music is not white noise; a synthetic buffer is not a stem.
    CI uses them only to assert *invariants* — direction and order of magnitude —
    never to reproduce a published figure. See docs/EXPERIMENTS.md for the real
    measurements and the material they came from.

USAGE
    python3 synth_fixtures.py als-chain OUTDIR [--versions N]
    python3 synth_fixtures.py als-pair  OUTDIR --edit {none,view,volume,rename}
    python3 synth_fixtures.py flp       OUTFILE
    python3 synth_fixtures.py buffer    OUTFILE --mb N --mutate {none,shift,global}

Standard library only, matching the constraint on experiments/.
"""

from __future__ import annotations

import argparse
import gzip
import os
import random
import struct

# ---------------------------------------------------------------------------
# Ableton .als — gzipped XML
# ---------------------------------------------------------------------------

SAMPLE_A = "735987__long__original-name.wav"
SAMPLE_B = "Renamed Sample.wav"


def _clip(cid: int, sample: str, start: float, end: float) -> str:
    return (
        f'<AudioClip Id="{cid}">'
        f'<Name Value="clip {cid}" />'
        f'<CurrentStart Value="{start}" />'
        f'<CurrentEnd Value="{end}" />'
        f'<Disabled Value="false" />'
        f'<SampleRef><FileRef>'
        f'<RelativePath Value="Samples/Imported/{sample}" />'
        f'<Path Value="Samples/Imported/{sample}" />'
        f"</FileRef></SampleRef>"
        f"</AudioClip>"
    )


def _track(
    tid: int,
    name: str,
    volume: float,
    clips: str,
    devices: str = "<AutoFilter /><Compressor2 />",
    view_scroll: int = 0,
) -> str:
    return (
        f'<AudioTrack Id="{tid}">'
        f'<Name><EffectiveName Value="{name}" /><UserName Value="" /></Name>'
        f'<Color Value="12" />'
        f"<DeviceChain>"
        f'<Mixer><Volume><Manual Value="{volume}" /></Volume>'
        f'<Pan><Manual Value="0" /></Pan>'
        f'<Speaker><Manual Value="true" /></Speaker></Mixer>'
        f"<MainSequencer><ClipTimeable><ArrangerAutomation><Events>"
        f"{clips}"
        f"</Events></ArrangerAutomation></ClipTimeable></MainSequencer>"
        f"<DeviceChain><Devices>{devices}</Devices></DeviceChain>"
        f"</DeviceChain>"
        # Pure view state. The extractor must ignore all of this — that is the
        # whole point of experiment 1 in docs/EXPERIMENTS.md.
        f'<ViewData Value="" />'
        f'<TrackScrollPosition Value="{view_scroll}" />'
        f'<ViewStateSessionTrackWidth Value="{93 + view_scroll}" />'
        f"</AudioTrack>"
    )


def als_xml(
    tempo: float = 120.0,
    volume: float = 0.794,
    sample: str = SAMPLE_A,
    clip_count: int = 40,
    view_scroll: int = 0,
    extra_track: bool = False,
) -> bytes:
    """Build a small but structurally real Live set."""
    clips = "".join(_clip(i, sample, float(i * 4), float(i * 4 + 4)) for i in range(clip_count))
    tracks = [
        _track(153, "Stem-mixing", volume, clips, view_scroll=view_scroll),
        _track(154, "Corpus metal", 0.5, _clip(900, "wood-hits.wav", 480.0, 495.5)),
    ]
    if extra_track:
        tracks.append(_track(155, "New Track", 0.85, ""))
    doc = (
        '<?xml version="1.0" encoding="UTF-8"?>'
        '<Ableton MajorVersion="5" MinorVersion="12.0_12049" Creator="Ableton Live 12.0.5">'
        "<LiveSet>"
        f"<Tracks>{''.join(tracks)}</Tracks>"
        "<MasterTrack><DeviceChain><Mixer>"
        f'<Tempo><Manual Value="{tempo}" /></Tempo>'
        "</Mixer></DeviceChain></MasterTrack>"
        f'<ScrollerPos Value="{view_scroll * 17}" />'
        f'<ViewStateFxSlotCount Value="{view_scroll % 7}" />'
        "</LiveSet></Ableton>"
    )
    return doc.encode("utf-8")


def write_als(path: str, payload: bytes) -> None:
    # mtime=0 so the fixture is byte-reproducible run to run.
    with open(path, "wb") as fh:
        with gzip.GzipFile(fileobj=fh, mode="wb", mtime=0) as gz:
            gz.write(payload)


def cmd_als_pair(out_dir: str, edit: str) -> None:
    os.makedirs(out_dir, exist_ok=True)
    write_als(os.path.join(out_dir, "old.als"), als_xml())
    if edit == "none":
        new = als_xml()
    elif edit == "view":
        # Scroll and zoom moved; nothing musical happened.
        new = als_xml(view_scroll=42)
    elif edit == "volume":
        new = als_xml(volume=0.525, view_scroll=42)
    elif edit == "rename":
        # One rename in the sample folder rewrites every clip that references it.
        new = als_xml(sample=SAMPLE_B, view_scroll=42)
    elif edit == "track":
        new = als_xml(extra_track=True)
    else:  # pragma: no cover - argparse constrains this
        raise SystemExit(f"unknown edit {edit!r}")
    write_als(os.path.join(out_dir, "new.als"), new)


# ---------------------------------------------------------------------------
# A chain of saves — the shape DAW saves actually have
# ---------------------------------------------------------------------------


def _body(rnd: random.Random, lines: int) -> list:
    """A large, repetitive, XML-ish body. Long verbatim runs, as in a real set."""
    out = []
    for i in range(lines):
        out.append(
            f'      <KeyMidi Id="{i}"><NoteName Value="C{i % 8}" />'
            f'<Velocity Value="{rnd.randrange(1, 128)}" />'
            f'<Time Value="{i * 0.25:.3f}" /><Curve Value="{rnd.random():.6f}" /></KeyMidi>'
        )
    return out


def cmd_als_chain(out_dir: str, versions: int, lines: int) -> None:
    """Write N sequential .als saves differing by ~0.2% of lines each.

    That churn rate is taken from the *shape* of experiment 1 (0.07-0.25% of
    lines per save). It is an input to the fixture, not a result measured from it.
    """
    os.makedirs(out_dir, exist_ok=True)
    rnd = random.Random(20260805)
    body = _body(rnd, lines)
    for v in range(versions):
        if v:
            # Scattered small edits, not one contiguous block.
            for _ in range(max(1, lines // 500)):
                i = rnd.randrange(lines)
                body[i] = body[i].replace('Curve Value="', f'Curve Value="{rnd.random():.6f}#')
        head = als_xml(tempo=120.0 + v, view_scroll=v).decode("utf-8")
        doc = head.replace("</LiveSet>", "<MidiKeyRanges>\n" + "\n".join(body) + "\n</MidiKeyRanges></LiveSet>")
        write_als(os.path.join(out_dir, "Project [%03d].als" % v), doc.encode("utf-8"))


# ---------------------------------------------------------------------------
# FL Studio .flp — a real FLhd/FLdt event stream
# ---------------------------------------------------------------------------


def _varint(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def cmd_flp(path: str, blob_bytes: int = 24000) -> None:
    """Minimal but real .flp: mostly opaque plugin state, as measured on real files."""
    rnd = random.Random(11)
    ev = bytearray()

    def text(eid: int, s: str) -> None:
        payload = s.encode("utf-16-le") + b"\x00\x00"
        ev.extend(bytes([eid]) + _varint(len(payload)) + payload)

    def blob(eid: int, size: int) -> None:
        ev.extend(bytes([eid]) + _varint(size) + bytes(rnd.getrandbits(8) for _ in range(size)))

    ev.extend(b"\x00\x01")          # id 0, 1-byte payload
    ev.extend(b"\x09\x01")          # id 9
    ev.extend(b"\x40\x02\x00")      # id 64, 2-byte payload
    ev.extend(b"\x42\x00\x00")      # id 66
    text(194, "Synthetic CI Fixture")
    text(207, "Wit CI")
    text(192, "Kick")
    text(196, "%FLStudioData%/Packs/Kick.wav")
    text(196, "%FLStudioData%/Packs/Snare.wav")
    for _ in range(6):
        blob(209, blob_bytes // 8)   # PluginParams
    for _ in range(2):
        blob(224, blob_bytes // 8)   # PluginSettings

    header = struct.pack("<hHH", 0, 4, 96)
    data = b"FLhd" + struct.pack("<I", len(header)) + header
    data += b"FLdt" + struct.pack("<I", len(ev)) + bytes(ev)
    with open(path, "wb") as fh:
        fh.write(data)


# ---------------------------------------------------------------------------
# Byte buffers for the chunking invariants
# ---------------------------------------------------------------------------


def cmd_buffer(path: str, mb: float, mutate: str, seed: int = 1234) -> None:
    rnd = random.Random(seed)
    n = int(mb * 1_000_000)
    buf = bytearray(rnd.getrandbits(8) for _ in range(n))
    if mutate == "shift":
        # A region moved in time: prepend, everything downstream shifts.
        buf = bytearray(rnd.getrandbits(8) for _ in range(9973)) + buf
    elif mutate == "global":
        # Every sample value is now a different number — the re-render wall.
        buf = bytearray((b + 1) & 0xFF for b in buf)
    elif mutate == "local":
        # One section re-rendered; the rest is untouched.
        start = n // 3
        for i in range(start, start + n // 20):
            buf[i] = (buf[i] + 1) & 0xFF
    with open(path, "wb") as fh:
        fh.write(bytes(buf))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("als-chain")
    p.add_argument("out_dir")
    p.add_argument("--versions", type=int, default=6)
    p.add_argument("--lines", type=int, default=20000)

    p = sub.add_parser("als-pair")
    p.add_argument("out_dir")
    p.add_argument("--edit", choices=["none", "view", "volume", "rename", "track"], required=True)

    p = sub.add_parser("flp")
    p.add_argument("out_file")

    p = sub.add_parser("buffer")
    p.add_argument("out_file")
    p.add_argument("--mb", type=float, default=1.0)
    p.add_argument("--mutate", choices=["none", "shift", "global", "local"], default="none")

    args = ap.parse_args()
    if args.cmd == "als-chain":
        cmd_als_chain(args.out_dir, args.versions, args.lines)
    elif args.cmd == "als-pair":
        cmd_als_pair(args.out_dir, args.edit)
    elif args.cmd == "flp":
        cmd_flp(args.out_file)
    elif args.cmd == "buffer":
        cmd_buffer(args.out_file, args.mb, args.mutate)


if __name__ == "__main__":
    main()
