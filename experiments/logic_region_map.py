#!/usr/bin/env python3
"""
Logic/GarageBand `ProjectData` -> which region sits on which track, at which bar.

WHAT THIS MEASURES
    The container walk (issue #25, `crates/wit-logic`) can already say "79 region
    records became 96". It cannot say WHICH region, or where it went. This script
    maps the two payloads that answer that, and reports hit rates so the claim is
    measured rather than asserted:

      gRuA ("AuRg", the region object) -- name, length in source frames, and a
      UUID that is unique per region AND stable across saves. That UUID is the
      Logic equivalent of Ableton's track `Id`: it is what makes a cross-save
      semantic diff possible at all.

      qSvE ("EvSq") -- the arrangement. Region POSITION is not in gRuA; it lives
      in placement events here. Each qSvE payload is a stream of 16-byte typed
      events, and an audio placement is a group headed by `24 00 00 00` carrying
      position, track number, and a link back to a region family.

    Measured with --scan over a 32-project Logic library, 132 saves:

      132/132   saves walked to clean EOF
      13606/13606  qSvE payloads an exact multiple of 16 (0 misaligned)
      10020/10020  region records decoded a name AND a distinct UUID
      5282/5282 placements resolved to a region family
      4995/5282 placements land on the 960-tick grid (the rest are legitimate --
                a region dragged with snap off sits at tick resolution)
      132/132   saves where every decoded track number is within that save's
                MetaData.plist NumberOfTracks
      265       marker hits rejected as collisions (see parse_event_stream)

    On the 10-save chain of one 31-track project, all 79 region UUIDs from the
    oldest save are still present in the newest, which adds exactly 17 more --
    that stability is what makes a cross-save diff possible.

USAGE
    Show the map for one save:
        python3 experiments/logic_region_map.py '/path/to/Project.logicx'

    What changed between two saves (a .logicx keeps ~10 in Project File Backups/):
        python3 experiments/logic_region_map.py --diff \\
            '/path/to/Project.logicx/Alternatives/000/Project File Backups/07' \\
            '/path/to/Project.logicx/Alternatives/000/Project File Backups/08'

    Hit rates across a whole library of .logicx/.band bundles:
        python3 experiments/logic_region_map.py --scan ~/Music/Logic

    Any argument may be a `ProjectData` file, a `.logicx`/`.band` bundle, or a
    numbered backup-slot directory. Add --json for machine-readable output.

WHAT THIS DOES NOT HANDLE
    - Region-instance disambiguation is INCOMPLETE, and the output says so. A
      placement links to a region FAMILY (one source file: "Angelic Vocal FX 03"
      and its .1/.2/.3 copies share one link value), not to an individual region
      record. On the measured project 23 of 35 families hold exactly as many
      region records as placements and pair 1:1; the other 12 hold more region
      records than placements -- 79 region objects for 56 placements -- so 23
      region objects exist that are not on the timeline. Those are reported as
      `unplaced`, never guessed at.
    - MIDI regions. Only audio placements (`24 00 00 00`) are decoded. The other
      six event types seen in a qSvE (0x88 0xaa 0x89 0x00 0x8a 0xbc) are counted
      but not interpreted. This is why GarageBand gets partial results: the
      three .band projects measured hold no gRuA records at all, so their
      placements report as `<unresolved family N>`. The container and the event
      grid generalise to GarageBand; the audio-region object was simply not
      exercised, because those projects are software-instrument only.
    - Tempo changes. Bar numbers assume 4/4 and a constant tempo, because the
      position field is in ticks (960 PPQ) and this script does not read the
      tempo or signature maps. A project with a meter change will show bar
      numbers that drift from what Logic displays.
    - Region length is in frames of the SOURCE FILE's sample rate, which is not
      necessarily the project rate (the measured project mixes 44100 Hz sources
      into a 48000 Hz project). This script reports frames, never seconds, so it
      cannot be wrong about which rate applies.
    - Fader moves, plugin parameters, automation. Those payloads are unmapped.
    - Writing. This script is read-only and never opens a DAW. Point it at a copy
      if you are at all unsure -- Logic migrates project files when it saves them.
"""

from __future__ import annotations

import argparse
import json
import plistlib
import struct
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------- #
# container framing
#
# Same constants as crates/wit-logic/src/frame.rs, deliberately named the same so
# the eventual Rust port is mechanical. Offsets below are RECORD-relative (from
# the start of the 36-byte record header); payload-relative = record - 0x24.
# --------------------------------------------------------------------------- #

MAGIC = b"\x23\x47\xc0\xab"
ROOT_HEADER_LEN = 0x18
RECORD_HEADER_LEN = 0x24
LENGTH_FIELD_OFFSET = 0x10
PAYLOAD_SIZE_FIELD_OFFSET = 0x1C

# `u32 @ record+0x08 = familyIndex << 18` -- the owning-object index. Its low 18
# bits are zero on every record measured (79/79).
IDX_FIELD_OFFSET = 0x08
IDX_SHIFT = 18

# A record count no real project approaches (the measured 1.3 MB file holds
# 2,069). Bounds the walk so a corrupt size field cannot spin forever.
MAX_RECORDS = 1_000_000

TAG_REGION = b"gRuA"  # "AuRg" reversed -- the region object
TAG_EVENTS = b"qSvE"  # "EvSq" reversed -- typed event streams, incl. placements

# --------------------------------------------------------------------------- #
# gRuA -- the region object
# --------------------------------------------------------------------------- #

REGION_LENGTH_OFFSET = 0x3A  # u32, length in frames of the source file
REGION_NAME_LEN_OFFSET = 0x6E  # u16 (= payload +0x4a, as in extract.rs)
REGION_NAME_OFFSET = 0x70

# The name is variable-length and the record is sized to fit it, padded to an
# even length: payload_size == 209 + nlen + (nlen & 1), exact on all 79 records
# of the measured save. Everything behind the name therefore SHIFTS with it --
# which is why the UUID is addressed from the padded name end, not from 0.
REGION_FIXED_SUFFIX_LEN = 133
REGION_UUID_OFFSET_IN_SUFFIX = 0x56
UUID_LEN = 16

# A name longer than this is a decode failure, not a name. The longest observed
# on real material is 26 bytes; extract.rs uses 100 for the same field.
MAX_REGION_NAME_LEN = 100

# --------------------------------------------------------------------------- #
# qSvE -- 16-byte typed events
# --------------------------------------------------------------------------- #

EVENT_LEN = 16
EVENT_TYPE_BYTE = 7  # a type code; 0x3f marks the single terminator event
EVENT_TYPE_TERMINATOR = 0x3F

PLACEMENT_MARKER = b"\x24\x00\x00\x00"
PLACEMENT_POSITION_OFFSET = 0x04  # u32, = REGION_TIME_ORIGIN + tick@960
PLACEMENT_EVENT_ID_OFFSET = 0x10  # u32
PLACEMENT_TRACK_OFFSET = 0x14  # u8, 1-based track number
PLACEMENT_LINK_OFFSET = 0x2C  # u32, link // 4 == familyIndex
PLACEMENT_GROUP_LEN = 0x30  # bytes of a group this script reads

TICKS_PER_QUARTER = 960
TICKS_PER_BAR = TICKS_PER_QUARTER * 4  # 4/4 only -- see WHAT THIS DOES NOT HANDLE
REGION_TIME_ORIGIN = 34560  # = 9 bars; regions use this origin, tempo/markers 38400

# An upper bound on a believable position, used to reject marker collisions that
# happen to carry a nonzero track byte. Across 132 saves of a 32-project library
# the highest genuine placement is bar 689 and the 99.9th percentile is bar 689;
# the single value above this bound was bar 821,376 on a 5-track project. At 4/4
# and 40 BPM, 10,000 bars is over 16 hours of music.
MAX_PLACEMENT_BAR = 10_000


class FormatError(Exception):
    """The bytes are not a `ProjectData` this script can read."""


@dataclass
class Record:
    tag: bytes
    offset: int
    payload_size: int


@dataclass
class Region:
    family: int
    name: str
    length_frames: int
    uuid: str
    offset: int


@dataclass
class Placement:
    track: int
    position: int
    family: int
    event_id: int

    @property
    def tick(self) -> int:
        return self.position - REGION_TIME_ORIGIN

    @property
    def bar(self) -> float:
        return self.tick / TICKS_PER_BAR + 1


@dataclass
class Song:
    regions: list[Region] = field(default_factory=list)
    placements: list[Placement] = field(default_factory=list)
    event_type_counts: dict[int, int] = field(default_factory=dict)
    event_stream_count: int = 0
    event_streams_misaligned: int = 0
    placement_markers_rejected: int = 0

    def families(self) -> dict[int, list[Region]]:
        out: dict[int, list[Region]] = defaultdict(list)
        for region in self.regions:
            out[region.family].append(region)
        return dict(out)


# --------------------------------------------------------------------------- #
# walking
# --------------------------------------------------------------------------- #


def walk_records(data: bytes) -> list[Record]:
    """Walk the record stream. Raises FormatError rather than guessing."""
    if len(data) < ROOT_HEADER_LEN:
        raise FormatError(
            "too short for a root header: %d bytes, need %d"
            % (len(data), ROOT_HEADER_LEN)
        )
    if data[:4] != MAGIC:
        raise FormatError("bad magic: %s" % data[:4].hex(" "))

    declared = struct.unpack_from("<I", data, LENGTH_FIELD_OFFSET)[0]
    expected = len(data) - ROOT_HEADER_LEN
    if declared != expected:
        raise FormatError(
            "root length mismatch: header declares %d, file has %d"
            % (declared, expected)
        )

    records: list[Record] = []
    pos = ROOT_HEADER_LEN
    while pos < len(data):
        if len(records) >= MAX_RECORDS:
            raise FormatError("more than %d records -- refusing to walk" % MAX_RECORDS)
        if pos + RECORD_HEADER_LEN > len(data):
            raise FormatError("record header at 0x%x runs past EOF" % pos)
        size = struct.unpack_from("<I", data, pos + PAYLOAD_SIZE_FIELD_OFFSET)[0]
        end = pos + RECORD_HEADER_LEN + size
        if end > len(data):
            raise FormatError(
                "record at 0x%x declares %d payload bytes, past EOF" % (pos, size)
            )
        records.append(Record(tag=data[pos : pos + 4], offset=pos, payload_size=size))
        pos = end
    return records


def _u32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def _format_uuid(raw: bytes) -> str:
    return "%s-%s-%s-%s-%s" % (
        raw[0:4].hex(),
        raw[4:6].hex(),
        raw[6:8].hex(),
        raw[8:10].hex(),
        raw[10:16].hex(),
    )


def parse_region(data: bytes, record: Record) -> Region | None:
    """Decode one gRuA record. Returns None if it does not decode cleanly."""
    base = record.offset
    total = RECORD_HEADER_LEN + record.payload_size
    if total < REGION_NAME_OFFSET + 2:
        return None

    name_len = struct.unpack_from("<H", data, base + REGION_NAME_LEN_OFFSET)[0]
    if name_len == 0 or name_len > MAX_REGION_NAME_LEN:
        return None

    padded = name_len + (name_len & 1)
    name_end = REGION_NAME_OFFSET + padded
    if name_end + REGION_FIXED_SUFFIX_LEN != total:
        # The size law did not hold, so every offset behind the name is a guess.
        return None

    raw_name = data[base + REGION_NAME_OFFSET : base + REGION_NAME_OFFSET + name_len]
    try:
        name = raw_name.decode("utf-8")
    except UnicodeDecodeError:
        return None
    if any(ord(ch) < 0x20 for ch in name):
        return None

    uuid_at = base + name_end + REGION_UUID_OFFSET_IN_SUFFIX
    return Region(
        family=_u32(data, base + IDX_FIELD_OFFSET) >> IDX_SHIFT,
        name=name,
        length_frames=_u32(data, base + REGION_LENGTH_OFFSET),
        uuid=_format_uuid(data[uuid_at : uuid_at + UUID_LEN]),
        offset=base,
    )


def parse_event_stream(
    data: bytes, record: Record
) -> tuple[list[Placement], dict[int, int], int]:
    """Decode one qSvE record's 16-byte event grid.

    Placement groups are located by their marker ON THIS PAYLOAD'S OWN EVENT
    GRID, bounded by this one record. That is framing-aware addressing, not the
    file-wide magic-signature scanning docs/FORMATS.md rules out: the walk above
    established the record boundaries first, and nothing here reads outside them.

    The marker is only four bytes, so it does collide with data inside other
    event types. Measured over 132 saves of a 32-project library: 5,547 marker
    hits, of which 265 are rejected -- 264 carrying track 0 (the byte is 1-based,
    so those are decisively not placements) and one at bar 821,376 on a 5-track
    project. Every one of the 5,282 that survive resolves to a real region
    family. Rejects are counted, not silently dropped.
    """
    start = record.offset + RECORD_HEADER_LEN
    payload = data[start : start + record.payload_size]
    if len(payload) % EVENT_LEN != 0:
        raise FormatError(
            "qSvE at 0x%x has a %d-byte payload, not a multiple of %d"
            % (record.offset, len(payload), EVENT_LEN)
        )

    placements: list[Placement] = []
    types: dict[int, int] = defaultdict(int)
    rejected = 0
    for i in range(0, len(payload), EVENT_LEN):
        types[payload[i + EVENT_TYPE_BYTE]] += 1
        if payload[i : i + 4] != PLACEMENT_MARKER:
            continue
        if i + PLACEMENT_GROUP_LEN > len(payload):
            # A group truncated by the end of the payload: count it as an event,
            # do not invent fields for it.
            rejected += 1
            continue
        track = payload[i + PLACEMENT_TRACK_OFFSET]
        position = _u32(payload, i + PLACEMENT_POSITION_OFFSET)
        max_position = REGION_TIME_ORIGIN + MAX_PLACEMENT_BAR * TICKS_PER_BAR
        if track < 1 or not REGION_TIME_ORIGIN <= position <= max_position:
            rejected += 1
            continue
        placements.append(
            Placement(
                track=track,
                position=position,
                family=_u32(payload, i + PLACEMENT_LINK_OFFSET) // 4,
                event_id=_u32(payload, i + PLACEMENT_EVENT_ID_OFFSET),
            )
        )
    return placements, dict(types), rejected


def parse(data: bytes, strict: bool = True) -> Song:
    """Parse one save.

    strict=True (the default, and what the single-file and --diff paths use)
    refuses a qSvE whose payload is not a multiple of 16, because every field
    behind it would then be a guess. strict=False counts the misalignment and
    carries on, so --scan can MEASURE the alignment rate instead of collapsing
    it into a generic read failure.
    """
    song = Song()
    for record in walk_records(data):
        if record.tag == TAG_REGION:
            region = parse_region(data, record)
            if region is not None:
                song.regions.append(region)
        elif record.tag == TAG_EVENTS:
            song.event_stream_count += 1
            try:
                placements, types, rejected = parse_event_stream(data, record)
            except FormatError:
                if strict:
                    raise
                song.event_streams_misaligned += 1
                continue
            song.placements.extend(placements)
            song.placement_markers_rejected += rejected
            for type_code, count in types.items():
                song.event_type_counts[type_code] = (
                    song.event_type_counts.get(type_code, 0) + count
                )
    song.placements.sort(key=lambda p: (p.position, p.track))
    return song


# --------------------------------------------------------------------------- #
# joining placements to regions
# --------------------------------------------------------------------------- #


def family_name(song: Song, family: int) -> tuple[str, int]:
    """The stem name of a placement's region family, and how many copies it holds.

    A placement links to a family (one source file), not to an individual region
    record. Where the family holds one region the name is exact. Where it holds
    more, the count is the caller's cue to phrase the result without claiming to
    know which copy it was.
    """
    regions = song.families().get(family)
    if not regions:
        return ("<unresolved family %d>" % family, 0)
    names = {r.name for r in regions}
    return (sorted(names, key=len)[0], len(names))


def family_label(song: Song, family: int) -> str:
    stem, copies = family_name(song, family)
    if copies <= 1:
        return stem
    return "%s (one of %d copies)" % (stem, copies)


def unplaced_regions(song: Song) -> list[Region]:
    """Region objects whose family has more records than it has placements."""
    per_family = defaultdict(int)
    for placement in song.placements:
        per_family[placement.family] += 1
    out: list[Region] = []
    for family, regions in sorted(song.families().items()):
        surplus = len(regions) - per_family.get(family, 0)
        if surplus > 0:
            out.extend(sorted(regions, key=lambda r: r.name)[-surplus:])
    return out


# --------------------------------------------------------------------------- #
# input resolution -- mirrors wit-cli's resolve_project_data
# --------------------------------------------------------------------------- #


def resolve_project_data(path: Path) -> Path:
    """Find the binary ProjectData under a file, bundle, or backup-slot path.

    Candidates are checked for the container magic rather than taken on name
    alone. A .band bundle ships a legacy XML `projectData` (lowercase p) at its
    top level alongside the real binary in Alternatives/000/, and macOS's
    case-insensitive filesystem makes the XML one match a `ProjectData` lookup.
    Reading it produced `bad magic: 3c 3f 78 6d` -- that is `<?xm`.
    """
    if path.is_file():
        return path
    if not path.is_dir():
        raise FormatError("no ProjectData found at %s" % path.name)

    candidates = [path / "ProjectData", path / "Alternatives" / "000" / "ProjectData"]
    found = [p for p in candidates if p.is_file()]
    for candidate in found:
        with candidate.open("rb") as handle:
            if handle.read(4) == MAGIC:
                return candidate
    if found:
        raise FormatError(
            "found %s but it is not a ProjectData container" % found[0].name
        )
    raise FormatError("no ProjectData found at %s" % path.name)


def load(path: Path, strict: bool = True) -> Song:
    return parse(resolve_project_data(path).read_bytes(), strict=strict)


# --------------------------------------------------------------------------- #
# reporting
# --------------------------------------------------------------------------- #


def format_bar(bar: float) -> str:
    return "%g" % bar


def report_map(song: Song) -> list[str]:
    families = song.families()
    lines = [
        "%d region objects in %d families, %d placements, %d event streams"
        % (len(song.regions), len(families), len(song.placements), song.event_stream_count),
        "",
        "ARRANGEMENT",
    ]
    for placement in song.placements:
        lines.append(
            "  track %-3d bar %-8s %s"
            % (placement.track, format_bar(placement.bar), family_label(song, placement.family))
        )

    unplaced = unplaced_regions(song)
    lines.extend(["", "REGION OBJECTS NOT ON THE TIMELINE (%d)" % len(unplaced)])
    for region in unplaced:
        lines.append("  %-34s %d frames" % (region.name, region.length_frames))

    ambiguous = sum(
        1
        for family, regions in families.items()
        if len({r.name for r in regions}) > 1
    )
    lines.extend(
        [
            "",
            "%d of %d families hold more than one distinct region name -- a placement "
            "in those cannot be attributed to one copy" % (ambiguous, len(families)),
        ]
    )
    return lines


def report_diff(old: Song, new: Song) -> list[str]:
    """Diff keyed on the region UUID, which is stable across saves."""
    lines: list[str] = []

    before = {r.uuid: r for r in old.regions}
    after = {r.uuid: r for r in new.regions}

    for uuid in sorted(set(after) - set(before)):
        lines.append("  region added: '%s'" % after[uuid].name)
    for uuid in sorted(set(before) - set(after)):
        lines.append("  region removed: '%s'" % before[uuid].name)
    for uuid in sorted(set(before) & set(after)):
        was, now = before[uuid], after[uuid]
        if was.name != now.name:
            lines.append("  region renamed: '%s' -> '%s'" % (was.name, now.name))
        if was.length_frames != now.length_frames:
            lines.append(
                "  region '%s' resized: %d -> %d frames"
                % (now.name, was.length_frames, now.length_frames)
            )

    # Placements are keyed per track, in bar order. A family with a single
    # region gives a name; a multi-copy family reports the move without
    # claiming which copy it was.
    def by_track(song: Song) -> dict[int, list[Placement]]:
        out: dict[int, list[Placement]] = defaultdict(list)
        for placement in song.placements:
            out[placement.track].append(placement)
        return out

    old_tracks, new_tracks = by_track(old), by_track(new)
    for track in sorted(set(old_tracks) | set(new_tracks)):
        was = old_tracks.get(track, [])
        now = new_tracks.get(track, [])
        old_bars = [p.position for p in was]
        new_bars = [p.position for p in now]
        if old_bars == new_bars:
            continue
        if len(was) == len(now):
            for a, b in zip(was, now):
                if a.position == b.position:
                    continue
                stem, copies = family_name(new, b.family)
                subject = "'%s'" % stem if copies <= 1 else "a '%s' region" % stem
                lines.append(
                    "  track %d: %s moved from bar %s to bar %s"
                    % (track, subject, format_bar(a.bar), format_bar(b.bar))
                )
        else:
            lines.append(
                "  track %d: %d placements -> %d placements"
                % (track, len(was), len(now))
            )

    return lines or ["  no region or placement change"]


def saves_in(bundle: Path) -> list[Path]:
    """Every ProjectData in a bundle: each alternative's current save + backups."""
    out = []
    for alternative in sorted((bundle / "Alternatives").glob("*")):
        current = alternative / "ProjectData"
        if current.is_file():
            out.append(current)
        backups = alternative / "Project File Backups"
        out.extend(sorted(p / "ProjectData" for p in sorted(backups.glob("*")))
                   if backups.is_dir() else [])
    return [p for p in out if p.is_file()]


def declared_track_count(save: Path) -> int | None:
    """NumberOfTracks from the sibling MetaData.plist, if there is one."""
    meta = save.parent / "MetaData.plist"
    if not meta.is_file():
        return None
    try:
        with meta.open("rb") as handle:
            value = plistlib.load(handle).get("NumberOfTracks")
    except (OSError, ValueError, plistlib.InvalidFileException):
        return None
    return value if isinstance(value, int) else None


def scan_library(root: Path) -> dict[str, int]:
    """Aggregate decode rates over a library. Never records a path or a name."""
    totals = dict.fromkeys(
        (
            "bundles",
            "saves",
            "saves_walked",
            "saves_unreadable",
            "regions",
            "regions_named",
            "regions_uuid_distinct",
            "event_streams",
            "event_streams_misaligned",
            "placements",
            "placement_markers_rejected",
            "placements_resolved_to_a_region",
            "placements_on_the_960_grid",
            "saves_with_declared_track_count",
            "saves_track_count_within_declared",
        ),
        0,
    )
    bundles = sorted(
        p
        for ext in ("*.logicx", "*.band")
        for p in root.rglob(ext)
        if p.is_dir() and (p / "Alternatives").is_dir()
    )
    for bundle in bundles:
        totals["bundles"] += 1
        for save in saves_in(bundle):
            totals["saves"] += 1
            try:
                song = parse(save.read_bytes(), strict=False)
            except (FormatError, OSError):
                totals["saves_unreadable"] += 1
                continue
            totals["saves_walked"] += 1
            totals["regions"] += len(song.regions)
            totals["regions_named"] += sum(1 for r in song.regions if r.name)
            totals["regions_uuid_distinct"] += len({r.uuid for r in song.regions})
            totals["event_streams"] += song.event_stream_count
            totals["event_streams_misaligned"] += song.event_streams_misaligned
            totals["placements"] += len(song.placements)
            totals["placement_markers_rejected"] += song.placement_markers_rejected
            families = song.families()
            totals["placements_resolved_to_a_region"] += sum(
                1 for p in song.placements if p.family in families
            )
            totals["placements_on_the_960_grid"] += sum(
                1
                for p in song.placements
                if p.tick >= 0 and p.tick % TICKS_PER_QUARTER == 0
            )
            declared = declared_track_count(save)
            if declared is not None:
                totals["saves_with_declared_track_count"] += 1
                highest = max((p.track for p in song.placements), default=0)
                if highest <= declared:
                    totals["saves_track_count_within_declared"] += 1
    return totals


# --------------------------------------------------------------------------- #
# cli
# --------------------------------------------------------------------------- #


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Map Logic/GarageBand ProjectData region payloads: which region is on "
            "which track, at which bar."
        ),
        epilog=(
            "Read-only. Any path may be a ProjectData file, a .logicx/.band bundle, "
            "or a numbered backup-slot directory."
        ),
    )
    parser.add_argument("path", nargs="?", type=Path, help="a save to map")
    parser.add_argument(
        "--diff",
        nargs=2,
        metavar=("OLD", "NEW"),
        type=Path,
        help="report what changed between two saves",
    )
    parser.add_argument(
        "--scan",
        metavar="DIR",
        type=Path,
        help="aggregate decode rates over a library of bundles (prints no paths)",
    )
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if sum(bool(x) for x in (args.path, args.diff, args.scan)) != 1:
        parser.error("give exactly one of: a path, --diff OLD NEW, or --scan DIR")

    try:
        if args.scan:
            totals = scan_library(args.scan)
            if args.json:
                print(json.dumps(totals, indent=2, sort_keys=True))
            else:
                for key in sorted(totals):
                    print("%-28s %d" % (key, totals[key]))
            return 0

        if args.diff:
            old, new = load(args.diff[0]), load(args.diff[1])
            lines = report_diff(old, new)
            if args.json:
                print(json.dumps({"changes": [line.strip() for line in lines]}, indent=2))
            else:
                print("\n".join(lines))
            return 0

        song = load(args.path)
        if args.json:
            print(
                json.dumps(
                    {
                        "regions": [vars(r) for r in song.regions],
                        "placements": [
                            dict(vars(p), bar=p.bar, tick=p.tick)
                            for p in song.placements
                        ],
                    },
                    indent=2,
                )
            )
        else:
            print("\n".join(report_map(song)))
        return 0

    except FormatError as exc:
        print("not readable: %s" % exc, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
