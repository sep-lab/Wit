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
      in placement events here. Each qSvE payload is a stream of 16-byte units,
      and an audio placement is a 48-byte group headed by `24 00 00 00` carrying
      position, track number, and a link back to a region family.

    Library-wide -- `--scan` over one person's 32-project Logic library, 132
    saves, 2026-08-16:

      132/132      saves walked to clean EOF
      13606/13606  qSvE payloads an exact multiple of 16 (0 misaligned)
      10020        gRuA records decoded (name, length, UUID). This is a COUNT,
                   not a rate: that scan counted only the records that decoded,
                   so a record that failed was invisible to it. The library-wide
                   decoded/seen rate is unmeasured until the re-run below.
      5282/5282    placements resolved to a region family
      4995/5282    placements on the 960-tick (quarter-note) grid; the rest are
                   INFERRED, not checked, to be regions placed with snap off
      132/132      saves where every decoded track number is within that save's
                   MetaData.plist NumberOfTracks
      265          marker hits rejected (see parse_event_stream)

    One project re-measured 2026-09-29 with the corrected counters -- the 10-save
    chain of the 31-track `You make my crazy!` fixture (docs/EXPERIMENTS.md §0):

      903/903      gRuA records seen were decoded
      1550/1550    qSvE payloads 16-byte aligned, each holding exactly one
                   terminator unit, always its last
      592/592      placements resolved; 602 marker hits, 10 rejected, all track 0
      592/592      placement groups whose three units carry byte +7 = 00/89/bc
      0/96         region UUIDs whose family index changes between saves

    Across that chain all 79 region UUIDs of the oldest save are still present in
    the newest, which adds exactly 17 more, and none of the 96 changes family
    index between saves -- that stability is what makes a cross-save diff
    possible.

    Library re-run, pending permission to read the library again:
        python3 experiments/logic_region_map.py --scan ~/Music/Logic

    Upstream: this corrects and extends §3, §8 and §8.1 of PROJECTDATA_FORMAT.md
    in jonkubis/LogicProFormatWriter (MIT), pinned at 1f77c5c3; docs/FORMATS.md
    says field by field where the two agree and where they differ.

USAGE
    Show the map for one save:
        python3 experiments/logic_region_map.py '/path/to/Project.logicx'

    What changed between two saves (a .logicx keeps ~10 in Project File Backups/):
        python3 experiments/logic_region_map.py --diff \\
            '/path/to/Project.logicx/Alternatives/000/Project File Backups/00' \\
            '/path/to/Project.logicx/Alternatives/000/Project File Backups/01'

    Hit rates across a whole library of .logicx/.band bundles (or one bundle):
        python3 experiments/logic_region_map.py --scan ~/Music/Logic

    Any argument may be a `ProjectData` file, a `.logicx`/`.band` bundle, or a
    numbered backup-slot directory. Add --json for machine-readable output.

WHAT THIS DOES NOT HANDLE
    - WHICH COPY. A placement links to a region FAMILY (one source file: "Angelic
      Vocal FX 03" and its .1/.2/.3 copies share one link value), not to an
      individual region record. On backup 00 of the measured chain, 20 of 35
      families hold a single region object (16 placed once, 4 not placed at all)
      and 15 hold two or more -- and those 15 carry 40 of the 56 placements. For
      those 40, which copy is on the timeline is not decodable: the map prints
      "one of N copies", and the diff says "a '<stem>' region". Families holding
      more region objects than placements are reported as a per-family surplus
      COUNT; the copies are never named, because which ones are off the timeline
      is exactly what cannot be decoded.
    - Moves inside one family that cancel out. The diff compares (track,
      position, family) placements, so two copies of the SAME family swapping
      positions is invisible to it. Two different families swapping is reported.
    - MIDI regions. Only audio placements (`24 00 00 00`) are decoded; MIDI
      placement heads (`20 00 00 00`, upstream spec §8.5) are not. Every 16-byte
      unit's byte +7 is counted (the map and --scan print the histogram) but not
      interpreted -- backup 00 of the measured chain carries 12 distinct values:
      00 3f 88 89 8a a3 a4 a7 aa b2 bb bc. This is why GarageBand gets partial
      results: the three .band projects in the 2026-08-16 scan hold no gRuA
      records at all, so their placements report as `<unresolved family N>`.
    - Pre-roll. A placement before bar 1 (position < 34560, the region time
      origin) is rejected and counted as `before_origin`, not decoded. None
      exists on the measured chain; the library count is unmeasured.
    - Tempo changes. Bar numbers assume 4/4 and a constant tempo, because the
      position field is in ticks (960 PPQ) and this script does not read the
      tempo or signature maps. A project with a meter change will show bar
      numbers that drift from what Logic displays.
    - Region length is in frames of the SOURCE FILE's sample rate, which is not
      necessarily the project rate (the measured project mixes 44100 Hz sources
      into a 48000 Hz project). This script reports frames, never seconds, so it
      cannot be wrong about which rate applies.
    - Name encoding. Names are decoded as UTF-8. Every name observed is 7-bit
      ASCII (903/903 on the measured chain), so ASCII and UTF-8 agree on all of
      them; how Logic encodes a non-ASCII name is untested.
    - Fader moves, plugin parameters, automation. Those payloads are unmapped.
    - Writing. This script is read-only and never opens a DAW. Point it at a copy
      if you are at all unsure -- Logic migrates project files when it saves them.
"""

from __future__ import annotations

import argparse
import json
import math
import plistlib
import struct
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from fractions import Fraction
from pathlib import Path

# --------------------------------------------------------------------------- #
# container framing
#
# Same constants as crates/wit-logic/src/frame.rs, deliberately named the same so
# the eventual Rust port is mechanical. Offsets below are RECORD-relative (from
# the start of the 36-byte record header); payload-relative = record - 0x24.
# Every value here is pinned against docs/FORMATS.md by
# tests/test_logic_region_map.py, and decoded from literal golden bytes there.
# --------------------------------------------------------------------------- #

MAGIC = b"\x23\x47\xc0\xab"
ROOT_HEADER_LEN = 0x18
RECORD_HEADER_LEN = 0x24
LENGTH_FIELD_OFFSET = 0x10
PAYLOAD_SIZE_FIELD_OFFSET = 0x1C

# `u32 @ record+0x08 = familyIndex << 18` -- the owning-object index. Its low 18
# bits are zero on every gRuA record of the measured chain (903/903).
IDX_FIELD_OFFSET = 0x08
IDX_SHIFT = 18

# A record count no real project approaches (the newest save of the measured
# chain, 1.27 MB, holds 2,069). Bounds the walk so a corrupt size field cannot
# spin forever.
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
# even length: payload_size == 209 + nlen + (nlen & 1), exact on all 903 records
# of the measured chain. Everything behind the name therefore SHIFTS with it --
# which is why the UUID is addressed from the padded name end, not from 0.
REGION_FIXED_SUFFIX_LEN = 133
REGION_UUID_OFFSET_IN_SUFFIX = 0x56
UUID_LEN = 16

# A name longer than this is a decode failure, not a name. The longest on the
# measured chain is 30 bytes; extract.rs uses 100 for the same field.
MAX_REGION_NAME_LEN = 100

# --------------------------------------------------------------------------- #
# qSvE -- a grid of 16-byte units
# --------------------------------------------------------------------------- #

EVENT_LEN = 16
# Byte +7 of every unit behaves like a type code [inferred]. It is counted, never
# interpreted. On a placement head it is also the HIGH BYTE of the u32 position
# at +0x04 -- 0x00 on every placement head measured.
EVENT_TYPE_BYTE = 7
# The last unit of every qSvE payload on the measured chain (1550/1550), and the
# only unit whose byte +7 is 0x3f there. Counted by --scan, not required.
TERMINATOR_EVENT = b"\xf1\x00\x00\x00\xff\xff\xff\x3f" + b"\x00" * 8

PLACEMENT_MARKER = b"\x24\x00\x00\x00"
PLACEMENT_POSITION_OFFSET = 0x04  # u32, = REGION_TIME_ORIGIN + tick@960
PLACEMENT_EVENT_ID_OFFSET = 0x10  # u32; NOT a per-placement key (see FORMATS.md)
PLACEMENT_TRACK_OFFSET = 0x14  # u8, 1-based track number
PLACEMENT_LINK_OFFSET = 0x2C  # u32, link // 4 == familyIndex
PLACEMENT_GROUP_LEN = 0x30  # bytes of a group this script reads: three units
# Byte +7 of the group's three units on every placement of the measured chain
# (592/592). Counted by --scan so the library re-run can test it; NOT used as a
# filter, because it has not been measured library-wide.
PLACEMENT_UNIT_TYPES = (0x00, 0x89, 0xBC)

TICKS_PER_QUARTER = 960
TICKS_PER_BAR = TICKS_PER_QUARTER * 4  # 4/4 only -- see WHAT THIS DOES NOT HANDLE
REGION_TIME_ORIGIN = 34560  # = 9 bars; regions use this origin, tempo/markers 38400

# An upper bound on a believable position, used to reject marker collisions that
# happen to carry a nonzero track byte. The 2026-08-16 library pass reported the
# highest accepted placement at bar 689 and one rejected hit at bar 821,376 on a
# 5-track project; that breakdown came from a one-off pass, not from this script,
# which now prints `placement_highest_bar` and the reject counts under --scan so
# the library re-run can confirm it (the measured chain's highest is bar 149).
# At 4/4 and 40 BPM, 10,000 bars is over 16 hours of music.
MAX_PLACEMENT_BAR = 10_000

REJECT_REASONS = ("truncated", "track_zero", "before_origin", "beyond_max_bar")


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
class EventStream:
    """What one qSvE payload held. Every marker hit is either a placement or a
    counted reject -- nothing is dropped silently."""

    placements: list[Placement] = field(default_factory=list)
    type_counts: Counter = field(default_factory=Counter)
    rejects: Counter = field(default_factory=Counter)
    rejected_head_types: Counter = field(default_factory=Counter)
    terminators: int = 0
    ends_in_terminator: bool = False
    placements_with_known_unit_types: int = 0
    head_gaps: Counter = field(default_factory=Counter)


@dataclass
class Song:
    regions: list[Region] = field(default_factory=list)
    placements: list[Placement] = field(default_factory=list)
    region_records_seen: int = 0
    event_type_counts: Counter = field(default_factory=Counter)
    event_stream_count: int = 0
    event_streams_misaligned: int = 0
    event_streams_with_one_terminator: int = 0
    event_streams_ending_in_terminator: int = 0
    placement_rejects: Counter = field(default_factory=Counter)
    rejected_marker_head_types: Counter = field(default_factory=Counter)
    placements_with_known_unit_types: int = 0
    placement_head_gaps: Counter = field(default_factory=Counter)

    @property
    def placement_markers_rejected(self) -> int:
        return sum(self.placement_rejects.values())

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
    """Decode one gRuA record. Returns None if it does not decode cleanly.

    The caller counts every gRuA record it sees, so a None here is a counted
    decode failure, never a silent drop.
    """
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


def parse_event_stream(data: bytes, record: Record) -> EventStream:
    """Decode one qSvE record's 16-byte unit grid.

    Placement groups are located by their marker ON THIS PAYLOAD'S OWN UNIT
    GRID, bounded by this one record. That is framing-aware addressing, not the
    file-wide magic-signature scanning docs/FORMATS.md rules out: the walk above
    established the record boundaries first, and nothing here reads outside them.

    The marker is only four bytes, so it does collide with data inside other
    units. A hit is rejected, and counted under the FIRST check it fails, as:
    `truncated` (the 48-byte group runs past the payload), `track_zero` (the
    track byte is 1-based, so 0 is decisively not a placement), `before_origin`
    (position < REGION_TIME_ORIGIN -- this also drops genuine pre-roll
    placements, see the module docstring) or `beyond_max_bar`.

    Measured: the 2026-08-16 library pass saw 5,547 marker hits and rejected
    265. The measured chain (2026-09-29, `--scan`) has 602 hits and 10 rejects,
    all `track_zero`, and on every one of them the head unit's byte +7 is 0x88
    (`rejected_marker_head_byte`), not the 0x00 all 592 accepted heads carry.
    That byte is also the high byte of the position, so it is a structural
    collision filter the Rust port can add once the library re-run confirms it.
    This script counts it and does not yet filter on it.
    """
    start = record.offset + RECORD_HEADER_LEN
    payload = data[start : start + record.payload_size]
    if len(payload) % EVENT_LEN != 0:
        raise FormatError(
            "qSvE at 0x%x has a %d-byte payload, not a multiple of %d"
            % (record.offset, len(payload), EVENT_LEN)
        )

    stream = EventStream()
    max_position = REGION_TIME_ORIGIN + MAX_PLACEMENT_BAR * TICKS_PER_BAR
    previous_head = None
    for i in range(0, len(payload), EVENT_LEN):
        stream.type_counts[payload[i + EVENT_TYPE_BYTE]] += 1
        if payload[i : i + EVENT_LEN] == TERMINATOR_EVENT:
            stream.terminators += 1
        if payload[i : i + 4] != PLACEMENT_MARKER:
            continue
        if i + PLACEMENT_GROUP_LEN > len(payload):
            # A group truncated by the end of the payload: count it, do not
            # invent fields for it.
            reason = "truncated"
        else:
            track = payload[i + PLACEMENT_TRACK_OFFSET]
            position = _u32(payload, i + PLACEMENT_POSITION_OFFSET)
            if track < 1:
                reason = "track_zero"
            elif position < REGION_TIME_ORIGIN:
                reason = "before_origin"
            elif position > max_position:
                reason = "beyond_max_bar"
            else:
                reason = None
        if reason is not None:
            stream.rejects[reason] += 1
            stream.rejected_head_types[payload[i + EVENT_TYPE_BYTE]] += 1
            continue
        unit_types = tuple(
            payload[i + unit + EVENT_TYPE_BYTE]
            for unit in range(0, PLACEMENT_GROUP_LEN, EVENT_LEN)
        )
        if unit_types == PLACEMENT_UNIT_TYPES:
            stream.placements_with_known_unit_types += 1
        if previous_head is not None:
            stream.head_gaps[i - previous_head] += 1
        previous_head = i
        stream.placements.append(
            Placement(
                track=track,
                position=position,
                family=_u32(payload, i + PLACEMENT_LINK_OFFSET) // 4,
                event_id=_u32(payload, i + PLACEMENT_EVENT_ID_OFFSET),
            )
        )
    stream.ends_in_terminator = payload[-EVENT_LEN:] == TERMINATOR_EVENT
    return stream


def parse(data: bytes, strict: bool = True) -> Song:
    """Parse one save.

    strict=True (the default, and what the single-file and --diff paths use)
    refuses a qSvE whose payload is not a multiple of 16, because every field
    behind it would then be a guess. strict=False counts the misalignment and
    carries on, so --scan can MEASURE the alignment rate instead of collapsing
    it into a generic read failure.

    Every gRuA record is counted in `region_records_seen` whether or not it
    decodes, so `len(song.regions) / song.region_records_seen` is a real rate.
    """
    song = Song()
    for record in walk_records(data):
        if record.tag == TAG_REGION:
            song.region_records_seen += 1
            region = parse_region(data, record)
            if region is not None:
                song.regions.append(region)
        elif record.tag == TAG_EVENTS:
            song.event_stream_count += 1
            try:
                stream = parse_event_stream(data, record)
            except FormatError:
                if strict:
                    raise
                song.event_streams_misaligned += 1
                continue
            song.placements.extend(stream.placements)
            song.event_type_counts.update(stream.type_counts)
            song.placement_rejects.update(stream.rejects)
            song.rejected_marker_head_types.update(stream.rejected_head_types)
            song.placements_with_known_unit_types += stream.placements_with_known_unit_types
            song.placement_head_gaps.update(stream.head_gaps)
            song.event_streams_with_one_terminator += stream.terminators == 1
            song.event_streams_ending_in_terminator += stream.ends_in_terminator
    song.placements.sort(key=lambda p: (p.position, p.track))
    return song


# --------------------------------------------------------------------------- #
# joining placements to regions
# --------------------------------------------------------------------------- #


def family_name(song: Song, family: int) -> tuple[str, int]:
    """The stem name of a placement's region family, and how many region
    objects it holds.

    A placement links to a family (one source file), not to an individual region
    record. Where the family holds one region object the name is exact. Where it
    holds more, the count is the caller's cue to phrase the result without
    claiming to know which copy it was. The stem is the shortest name, ties
    broken by the name itself, so the choice never depends on hash order.
    """
    regions = song.families().get(family)
    if not regions:
        return ("<unresolved family %d>" % family, 0)
    names = {r.name for r in regions}
    return (min(names, key=lambda n: (len(n), n)), len(regions))


def family_label(song: Song, family: int) -> str:
    stem, copies = family_name(song, family)
    if copies <= 1:
        return stem
    return "%s (one of %d copies)" % (stem, copies)


def family_subject(song: Song, family: int) -> str:
    """How a diff line names a family: exactly, or without picking a copy."""
    stem, copies = family_name(song, family)
    return "'%s'" % stem if copies <= 1 else "a '%s' region" % stem


def family_surplus(song: Song) -> list[tuple[str, int, int]]:
    """Families holding more region objects than placements.

    Returns (stem, region objects, placements) per family. The surplus is a
    COUNT: a placement names a family, not a copy, so which of a family's copies
    are the ones off the timeline is not decodable, and none is named.
    """
    placed = Counter(p.family for p in song.placements)
    out = []
    for family, regions in sorted(song.families().items()):
        if len(regions) > placed.get(family, 0):
            stem, _ = family_name(song, family)
            out.append((stem, len(regions), placed.get(family, 0)))
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


def format_bar(position: int) -> str:
    """The bar a placement position falls on, exactly -- never rounded.

    Whole bars print as `9`. A fraction prints as a decimal when three places
    hold it exactly (`55.75`), otherwise as a mixed fraction (`66 5/6`,
    `100 1/3840`), so a one-tick move can never print as no move at all.
    """
    bar = Fraction(position - REGION_TIME_ORIGIN, TICKS_PER_BAR) + 1
    if bar.denominator == 1:
        return str(bar.numerator)
    thousandths = bar * 1000
    if thousandths.denominator == 1:
        whole, part = divmod(thousandths.numerator, 1000)
        return ("%d.%03d" % (whole, part)).rstrip("0")
    whole = math.floor(bar)
    rest = bar - whole
    return "%d %d/%d" % (whole, rest.numerator, rest.denominator)


def _histogram(counts: Counter, key_format: str) -> str:
    return " ".join("%s:%d" % (key_format % k, counts[k]) for k in sorted(counts))


def report_map(song: Song) -> list[str]:
    families = song.families()
    lines = [
        "%d of %d region records decoded, in %d families; %d placements; %d event streams"
        % (
            len(song.regions),
            song.region_records_seen,
            len(families),
            len(song.placements),
            song.event_stream_count,
        ),
        "",
        "ARRANGEMENT",
    ]
    for placement in song.placements:
        lines.append(
            "  track %-3d bar %-10s %s"
            % (
                placement.track,
                format_bar(placement.position),
                family_label(song, placement.family),
            )
        )

    multi = {f for f, regions in families.items() if len(regions) > 1}
    in_multi = sum(1 for p in song.placements if p.family in multi)
    exact = sum(
        1 for p in song.placements if p.family in families and p.family not in multi
    )
    lines.extend(
        [
            "",
            "WHICH COPY",
            "  %d of %d placements are in a family holding one region object: named exactly"
            % (exact, len(song.placements)),
            "  %d of %d placements are in the %d of %d families holding 2+ region objects:"
            % (in_multi, len(song.placements), len(multi), len(families)),
            "  which copy each one is cannot be decoded, so they print as 'one of N copies'",
        ]
    )

    surplus = family_surplus(song)
    lines.extend(
        ["", "FAMILIES WITH MORE REGION OBJECTS THAN PLACEMENTS (%d)" % len(surplus)]
    )
    for stem, records, placed in surplus:
        lines.append(
            "  %-34s %d region object(s), %d placed: %d not on the timeline"
            % (stem, records, placed, records - placed)
        )
    if surplus:
        lines.append(
            "  (a placement names a family, not a copy: where some of a family is"
            " placed, which copies are the unplaced ones is not decodable)"
        )

    units = sum(song.event_type_counts.values())
    lines.extend(
        [
            "",
            "EVENT GRID",
            "  %d 16-byte units; byte +7 values (%d distinct): %s"
            % (units, len(song.event_type_counts), _histogram(song.event_type_counts, "%02x")),
            "  placement groups whose three units carry byte +7 = 00/89/bc: %d of %d"
            % (song.placements_with_known_unit_types, len(song.placements)),
            "  gaps between consecutive placement heads, bytes: %s"
            % (_histogram(song.placement_head_gaps, "%d") or "none"),
            "  placement +0x10 values: %d distinct over %d placements on %d tracks,"
            " %d distinct (value, track) pairs"
            % (
                len({p.event_id for p in song.placements}),
                len(song.placements),
                len({p.track for p in song.placements}),
                len({(p.event_id, p.track) for p in song.placements}),
            ),
            "  placement markers rejected: %d (%s); their head byte +7: %s"
            % (
                song.placement_markers_rejected,
                ", ".join("%s %d" % (r, song.placement_rejects[r]) for r in REJECT_REASONS),
                _histogram(song.rejected_marker_head_types, "%02x") or "none",
            ),
        ]
    )
    return lines


def placement_changes(old: Song, new: Song) -> list[str]:
    """What happened on the timeline between two saves.

    Placements are compared as (track, position, family) multisets. Exact
    matches cancel. What is left is grouped by family: a family with exactly one
    placement gone and exactly one placement new is reported as a MOVE, because
    that pairing is unambiguous. Everything else is reported as placements added
    and removed -- a pairing that would need to know which copy is which is never
    guessed. Output is in timeline order.
    """
    def keyed(song: Song) -> Counter:
        return Counter((p.track, p.position, p.family) for p in song.placements)

    before, after = keyed(old), keyed(new)
    gone: dict[int, list[tuple[int, int]]] = defaultdict(list)
    came: dict[int, list[tuple[int, int]]] = defaultdict(list)
    for (track, position, family), n in (before - after).items():
        gone[family].extend([(track, position)] * n)
    for (track, position, family), n in (after - before).items():
        came[family].extend([(track, position)] * n)

    events: list[tuple[tuple[int, int, int], str]] = []
    for family in sorted(set(gone) | set(came)):
        was, now = sorted(gone.get(family, [])), sorted(came.get(family, []))
        if len(was) == 1 and len(now) == 1:
            (t0, p0), (t1, p1) = was[0], now[0]
            subject = family_subject(new, family)
            if t0 == t1:
                line = "  track %d: %s moved from bar %s to bar %s" % (
                    t0, subject, format_bar(p0), format_bar(p1)
                )
            else:
                line = "  %s moved from track %d bar %s to track %d bar %s" % (
                    subject, t0, format_bar(p0), t1, format_bar(p1)
                )
            events.append(((p0, t0, 0), line))
            continue
        for track, position in was:
            events.append(
                (
                    (position, track, 1),
                    "  track %d: placement removed: %s at bar %s"
                    % (track, family_subject(old, family), format_bar(position)),
                )
            )
        for track, position in now:
            events.append(
                (
                    (position, track, 2),
                    "  track %d: placement added: %s at bar %s"
                    % (track, family_subject(new, family), format_bar(position)),
                )
            )
    events.sort()
    return [line for _, line in events]


def report_diff(old: Song, new: Song) -> list[str]:
    """Diff keyed on the region UUID, which is stable across saves."""
    lines: list[str] = []

    before = {r.uuid: r for r in old.regions}
    after = {r.uuid: r for r in new.regions}

    for uuid in sorted(set(after) - set(before), key=lambda u: (after[u].name, u)):
        lines.append("  region added: '%s'" % after[uuid].name)
    for uuid in sorted(set(before) - set(after), key=lambda u: (before[u].name, u)):
        lines.append("  region removed: '%s'" % before[uuid].name)
    for uuid in sorted(set(before) & set(after), key=lambda u: (after[u].name, u)):
        was, now = before[uuid], after[uuid]
        if was.name != now.name:
            lines.append("  region renamed: '%s' -> '%s'" % (was.name, now.name))
        if was.length_frames != now.length_frames:
            lines.append(
                "  region '%s' resized: %d -> %d frames"
                % (now.name, was.length_frames, now.length_frames)
            )

    lines.extend(placement_changes(old, new))
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


SCAN_COUNTERS = (
    "bundles",
    "saves",
    "saves_walked",
    "saves_unreadable",
    "region_records_seen",
    "region_records_decoded",
    "region_uuids_distinct_within_save",
    "event_streams",
    "event_streams_misaligned",
    "event_streams_with_exactly_one_terminator",
    "event_streams_ending_in_terminator",
    "placement_markers_seen",
    "placements",
    "placements_resolved_to_a_region",
    "placements_on_the_960_grid",
    "placements_with_unit_types_00_89_bc",
    "placement_highest_bar",
    "placement_markers_rejected",
    *("placement_markers_rejected_%s" % r for r in REJECT_REASONS),
    "saves_with_declared_track_count",
    "saves_track_count_within_declared",
    "region_uuids_seen_in_2plus_saves_of_a_bundle",
    "region_uuids_changing_family_across_saves",
)


def scan_library(root: Path) -> dict:
    """Aggregate decode rates over a library. Never records a path or a name.

    Returns the SCAN_COUNTERS as ints, plus three histograms: `event_type_byte`
    (byte +7 of every 16-byte unit, keyed `0x..`), `rejected_marker_head_byte`
    (byte +7 of the head unit of every rejected marker hit) and
    `placement_head_gap_bytes` (distance between consecutive placement heads in
    one payload).
    """
    totals: dict = dict.fromkeys(SCAN_COUNTERS, 0)
    type_bytes: Counter = Counter()
    rejected_heads: Counter = Counter()
    head_gaps: Counter = Counter()
    def is_bundle(p: Path) -> bool:
        return p.suffix in (".logicx", ".band") and (p / "Alternatives").is_dir()

    # A bundle given directly is scanned as a library of one.
    bundles = [root] if is_bundle(root) else sorted(
        p for ext in ("*.logicx", "*.band") for p in root.rglob(ext) if is_bundle(p)
    )
    for bundle in bundles:
        totals["bundles"] += 1
        family_of: dict[str, set[int]] = defaultdict(set)
        saves_holding: Counter = Counter()
        for save in saves_in(bundle):
            totals["saves"] += 1
            try:
                song = parse(save.read_bytes(), strict=False)
            except (FormatError, OSError):
                totals["saves_unreadable"] += 1
                continue
            totals["saves_walked"] += 1
            totals["region_records_seen"] += song.region_records_seen
            totals["region_records_decoded"] += len(song.regions)
            totals["region_uuids_distinct_within_save"] += len({r.uuid for r in song.regions})
            totals["event_streams"] += song.event_stream_count
            totals["event_streams_misaligned"] += song.event_streams_misaligned
            totals["event_streams_with_exactly_one_terminator"] += (
                song.event_streams_with_one_terminator
            )
            totals["event_streams_ending_in_terminator"] += (
                song.event_streams_ending_in_terminator
            )
            totals["placement_markers_seen"] += (
                len(song.placements) + song.placement_markers_rejected
            )
            totals["placements"] += len(song.placements)
            totals["placement_markers_rejected"] += song.placement_markers_rejected
            for reason in REJECT_REASONS:
                totals["placement_markers_rejected_%s" % reason] += (
                    song.placement_rejects[reason]
                )
            families = song.families()
            totals["placements_resolved_to_a_region"] += sum(
                1 for p in song.placements if p.family in families
            )
            totals["placements_on_the_960_grid"] += sum(
                1 for p in song.placements if p.tick % TICKS_PER_QUARTER == 0
            )
            totals["placements_with_unit_types_00_89_bc"] += (
                song.placements_with_known_unit_types
            )
            totals["placement_highest_bar"] = max(
                [totals["placement_highest_bar"]]
                + [p.tick // TICKS_PER_BAR + 1 for p in song.placements]
            )
            type_bytes.update(song.event_type_counts)
            rejected_heads.update(song.rejected_marker_head_types)
            head_gaps.update(song.placement_head_gaps)
            for region in song.regions:
                family_of[region.uuid].add(region.family)
            saves_holding.update({r.uuid for r in song.regions})
            declared = declared_track_count(save)
            if declared is not None:
                totals["saves_with_declared_track_count"] += 1
                highest = max((p.track for p in song.placements), default=0)
                if highest <= declared:
                    totals["saves_track_count_within_declared"] += 1
        totals["region_uuids_seen_in_2plus_saves_of_a_bundle"] += sum(
            1 for n in saves_holding.values() if n > 1
        )
        totals["region_uuids_changing_family_across_saves"] += sum(
            1 for families in family_of.values() if len(families) > 1
        )
    totals["event_type_byte"] = {"0x%02x" % k: type_bytes[k] for k in sorted(type_bytes)}
    totals["rejected_marker_head_byte"] = {
        "0x%02x" % k: rejected_heads[k] for k in sorted(rejected_heads)
    }
    totals["placement_head_gap_bytes"] = {str(k): head_gaps[k] for k in sorted(head_gaps)}
    return totals


def format_scan(totals: dict) -> list[str]:
    lines = ["%-44s %d" % (key, totals[key]) for key in SCAN_COUNTERS]
    lines.append("event_type_byte (%d distinct)" % len(totals["event_type_byte"]))
    lines.extend("  %-42s %d" % kv for kv in totals["event_type_byte"].items())
    lines.append("rejected_marker_head_byte")
    lines.extend("  %-42s %d" % kv for kv in totals["rejected_marker_head_byte"].items())
    lines.append("placement_head_gap_bytes")
    lines.extend("  %-42s %d" % kv for kv in totals["placement_head_gap_bytes"].items())
    return lines


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
                print("\n".join(format_scan(totals)))
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
                        "region_records_seen": song.region_records_seen,
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
