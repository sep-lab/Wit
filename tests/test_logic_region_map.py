"""
Tests for experiments/logic_region_map.py.

Every fixture is generated in code. No `ProjectData`, `.logicx` or `.band` byte
is ever committed (CONTRIBUTING.md).

Two kinds of fixture, on purpose:

- The GOLDEN bytes (first section) are written out at literal offsets, with no
  reference to any `lrm.*` constant. They are the independent check: if a
  constant in the parser drifts, the parser stops reading them correctly and
  those tests fail. The builders further down reuse the parser's constants for
  convenience, so on their own they could never catch an offset drifting -- a
  mutation check on the original version of this file proved exactly that.
- The builders exercise behaviour (diff pairing, rejects, --scan counters) on
  top of an offset table the golden tests have already pinned.
"""

from __future__ import annotations

import re
import struct
from pathlib import Path

import logic_region_map as lrm
import pytest

FORMATS_MD = Path(__file__).resolve().parent.parent / "docs" / "FORMATS.md"

# --------------------------------------------------------------------------- #
# golden bytes -- literal offsets, independent of the module's constants
# --------------------------------------------------------------------------- #

GOLDEN_UUID_SNARE = "2d5ee052-641c-11f1-854a-205d0d40b51c"
GOLDEN_UUID_KICK = "a17c3e08-0b2d-11f1-9c3e-6e0b9a1f4d27"


def golden_region_snare() -> bytes:
    """gRuA, odd-length name: 'Snare' (5 bytes, padded to 6), family 7.

    251 bytes = 0x70 + 6 + 133. payload_size 215 = 209 + 5 + 1. The UUID sits
    at 0x70 + 6 + 0x56 = 0xCC.
    """
    rec = bytearray(251)
    rec[0x00:0x04] = b"gRuA"
    rec[0x08:0x0C] = bytes.fromhex("00 00 1c 00")  # 7 << 18 = 0x001c0000
    rec[0x1C:0x20] = bytes.fromhex("d7 00 00 00")  # payload size 215
    rec[0x3A:0x3E] = bytes.fromhex("c5 a5 02 00")  # 173509 frames
    rec[0x6E:0x70] = bytes.fromhex("05 00")  # name length 5
    rec[0x70:0x75] = b"Snare"  # + one pad byte at 0x75
    rec[0xCC:0xDC] = bytes.fromhex("2d5ee052 641c 11f1 854a 205d0d40b51c")
    return bytes(rec)


def golden_region_kick() -> bytes:
    """gRuA, even-length name: 'Kick' (4 bytes, no pad), family 9.

    249 bytes = 0x70 + 4 + 133. payload_size 213 = 209 + 4. UUID at 0xCA.
    """
    rec = bytearray(249)
    rec[0x00:0x04] = b"gRuA"
    rec[0x08:0x0C] = bytes.fromhex("00 00 24 00")  # 9 << 18 = 0x00240000
    rec[0x1C:0x20] = bytes.fromhex("d5 00 00 00")  # payload size 213
    rec[0x3A:0x3E] = bytes.fromhex("c0 96 03 00")  # 235200 frames
    rec[0x6E:0x70] = bytes.fromhex("04 00")
    rec[0x70:0x74] = b"Kick"
    rec[0xCA:0xDA] = bytes.fromhex("a17c3e08 0b2d 11f1 9c3e 6e0b9a1f4d27")
    return bytes(rec)


def golden_event_stream() -> bytes:
    """qSvE holding one 48-byte audio placement group plus the terminator.

    Track 3, bar 55.75 (position 34560 + 54.75 * 3840 = 244800 = 0x0003bc40),
    event id 0x64, link 0x1c (= family 7 * 4). The three units carry byte +7 =
    00 / 89 / bc, as every real placement group measured does.
    """
    payload = bytes.fromhex(
        "24 00 00 00 40 bc 03 00 00 00 00 00 00 00 00 00"  # +0x00 marker, position
        "64 00 00 00 03 00 00 89 00 00 00 00 00 00 00 00"  # +0x10 event id, +0x14 track
        "00 00 00 00 00 00 00 bc 00 00 00 00 1c 00 00 00"  # +0x2c link
        "f1 00 00 00 ff ff ff 3f 00 00 00 00 00 00 00 00"  # terminator
    )
    header = bytearray(0x24)
    header[0x00:0x04] = b"qSvE"
    header[0x08:0x0C] = bytes.fromhex("00 00 04 00")
    header[0x1C:0x20] = bytes.fromhex("40 00 00 00")  # payload size 64
    return bytes(header) + payload


def golden_container() -> bytes:
    body = golden_region_snare() + golden_region_kick() + golden_event_stream()
    root = bytearray(0x18)
    root[0x00:0x04] = bytes.fromhex("23 47 c0 ab")
    root[0x04:0x06] = bytes.fromhex("d0 09")
    root[0x10:0x14] = bytes.fromhex("58 02 00 00")  # 600 body bytes
    assert len(body) == 600
    return bytes(root) + body


def test_golden_bytes_decode_both_regions_exactly():
    song = lrm.parse(golden_container())
    assert song.region_records_seen == 2
    assert [
        (r.family, r.name, r.length_frames, r.uuid, r.offset) for r in song.regions
    ] == [
        (7, "Snare", 173509, GOLDEN_UUID_SNARE, 0x18),
        (9, "Kick", 235200, GOLDEN_UUID_KICK, 0x18 + 251),
    ]


def test_golden_bytes_decode_the_placement_exactly():
    song = lrm.parse(golden_container())
    assert [(p.track, p.position, p.family, p.event_id) for p in song.placements] == [
        (3, 244800, 7, 0x64)
    ]
    placement = song.placements[0]
    assert placement.tick == 210240
    assert lrm.format_bar(placement.position) == "55.75"
    assert lrm.family_label(song, placement.family) == "Snare"


def test_golden_bytes_decode_the_unit_grid_exactly():
    song = lrm.parse(golden_container())
    assert dict(song.event_type_counts) == {0x00: 1, 0x89: 1, 0xBC: 1, 0x3F: 1}
    assert (
        song.event_stream_count,
        song.event_streams_with_one_terminator,
        song.event_streams_ending_in_terminator,
        song.placements_with_known_unit_types,
        song.placement_markers_rejected,
    ) == (1, 1, 1, 1, 0)


# Each constant, its value as documented in docs/FORMATS.md, and the text the
# doc must contain for that value. A drift in either the code or the doc fails.
DOCUMENTED = [
    ("LENGTH_FIELD_OFFSET", 0x10, "`u32` length at `+0x10`"),
    ("PAYLOAD_SIZE_FIELD_OFFSET", 0x1C, "size field at `+0x1c`"),
    ("IDX_FIELD_OFFSET", 0x08, "| `+0x08` | u32 | `familyIndex << 18`"),
    ("IDX_SHIFT", 18, "`familyIndex << 18`"),
    ("REGION_LENGTH_OFFSET", 0x3A, "| `+0x3A` | u32 | Region length"),
    ("REGION_NAME_LEN_OFFSET", 0x6E, "| `+0x6E` | u16 | Name length"),
    ("REGION_NAME_OFFSET", 0x70, "| `+0x70` |"),
    ("REGION_FIXED_SUFFIX_LEN", 133, "a constant 133 bytes"),
    ("REGION_UUID_OFFSET_IN_SUFFIX", 0x56, "| name end `+0x56` | 16 B | Region UUID"),
    ("PLACEMENT_POSITION_OFFSET", 0x04, "| `+0x04` | u32 | Position"),
    ("PLACEMENT_EVENT_ID_OFFSET", 0x10, "| `+0x10` | u32 |"),
    ("PLACEMENT_TRACK_OFFSET", 0x14, "| `+0x14` | u8 | **Track number, 1-based**"),
    ("PLACEMENT_LINK_OFFSET", 0x2C, "| `+0x2c` | u32 | Region link"),
    ("REGION_TIME_ORIGIN", 34560, "Position = `34560 + tick@960`"),
    ("PLACEMENT_MARKER", b"\x24\x00\x00\x00", "headed by `24 00 00 00`"),
]


@pytest.mark.parametrize("name,value,doc_text", DOCUMENTED, ids=[d[0] for d in DOCUMENTED])
def test_each_offset_equals_its_documented_value(name, value, doc_text):
    assert getattr(lrm, name) == value
    assert doc_text in FORMATS_MD.read_text(encoding="utf-8"), (
        "docs/FORMATS.md no longer says %r -- code and doc have drifted" % doc_text
    )


def test_the_non_offset_constants_are_the_measured_ones():
    assert (lrm.MAGIC, lrm.ROOT_HEADER_LEN, lrm.RECORD_HEADER_LEN) == (
        b"\x23\x47\xc0\xab",
        24,
        36,
    )
    assert (lrm.TAG_REGION, lrm.TAG_EVENTS, lrm.UUID_LEN) == (b"gRuA", b"qSvE", 16)
    assert (lrm.EVENT_LEN, lrm.EVENT_TYPE_BYTE, lrm.PLACEMENT_GROUP_LEN) == (16, 7, 48)
    assert lrm.TERMINATOR_EVENT == bytes.fromhex("f1000000ffffff3f") + bytes(8)
    assert lrm.PLACEMENT_UNIT_TYPES == (0x00, 0x89, 0xBC)
    assert (lrm.TICKS_PER_QUARTER, lrm.TICKS_PER_BAR) == (960, 3840)
    assert (lrm.MAX_REGION_NAME_LEN, lrm.MAX_PLACEMENT_BAR) == (100, 10_000)


# --------------------------------------------------------------------------- #
# builders -- convenience on top of the offsets pinned above
# --------------------------------------------------------------------------- #

TERMINATOR = b"\xf1\x00\x00\x00\xff\xff\xff\x3f" + b"\x00" * 8


def build_record(tag: bytes, payload: bytes, idx: int = 0) -> bytes:
    header = bytearray(lrm.RECORD_HEADER_LEN)
    header[0:4] = tag
    struct.pack_into("<I", header, lrm.IDX_FIELD_OFFSET, idx)
    struct.pack_into("<I", header, lrm.PAYLOAD_SIZE_FIELD_OFFSET, len(payload))
    return bytes(header) + payload


def build_container(records: list[bytes], version: bytes = b"\xd0\x09") -> bytes:
    body = b"".join(records)
    head = bytearray(lrm.ROOT_HEADER_LEN)
    head[0:4] = lrm.MAGIC
    head[4:6] = version
    struct.pack_into("<I", head, lrm.LENGTH_FIELD_OFFSET, len(body))
    return bytes(head) + body


def build_region(name: str, length_frames: int, uuid: bytes, family: int = 0) -> bytes:
    """A gRuA record. Payload size follows the measured law: 209 + padded name."""
    raw = name.encode("utf-8")
    name_len = len(raw)
    padded = name_len + (name_len & 1)

    # Offsets here are PAYLOAD-relative; the parser's constants are
    # RECORD-relative, so each is the parser's constant minus RECORD_HEADER_LEN.
    length_at = lrm.REGION_LENGTH_OFFSET - lrm.RECORD_HEADER_LEN
    name_len_at = lrm.REGION_NAME_LEN_OFFSET - lrm.RECORD_HEADER_LEN
    name_at = lrm.REGION_NAME_OFFSET - lrm.RECORD_HEADER_LEN
    uuid_at = name_at + padded + lrm.REGION_UUID_OFFSET_IN_SUFFIX

    payload = bytearray(name_at + padded + lrm.REGION_FIXED_SUFFIX_LEN)
    struct.pack_into("<I", payload, length_at, length_frames)
    struct.pack_into("<H", payload, name_len_at, name_len)
    payload[name_at : name_at + name_len] = raw
    payload[uuid_at : uuid_at + lrm.UUID_LEN] = uuid
    return build_record(lrm.TAG_REGION, bytes(payload), idx=family << lrm.IDX_SHIFT)


def build_placement(track: int, bar: float, family: int, event_id: int = 0x58) -> bytes:
    """One 48-byte placement group (three 16-byte units, byte +7 = 00/89/bc)."""
    event = bytearray(lrm.PLACEMENT_GROUP_LEN)
    event[0:4] = lrm.PLACEMENT_MARKER
    position = lrm.REGION_TIME_ORIGIN + round((bar - 1) * lrm.TICKS_PER_BAR)
    struct.pack_into("<I", event, lrm.PLACEMENT_POSITION_OFFSET, position)
    struct.pack_into("<I", event, lrm.PLACEMENT_EVENT_ID_OFFSET, event_id)
    event[lrm.PLACEMENT_TRACK_OFFSET] = track
    event[0x10 + lrm.EVENT_TYPE_BYTE] = 0x89
    event[0x20 + lrm.EVENT_TYPE_BYTE] = 0xBC
    struct.pack_into("<I", event, lrm.PLACEMENT_LINK_OFFSET, family * 4)
    return bytes(event)


def build_event_stream(events: list[bytes]) -> bytes:
    return build_record(lrm.TAG_EVENTS, b"".join(events) + TERMINATOR)


def uuid_bytes(seed: int) -> bytes:
    return bytes([seed]) + b"\x5e\xe0\x52\x64\x1c\x11\xf1\x85\x4a\x20\x5d\x0d\x40\xb5\x1c"


def song_with(regions, placements) -> lrm.Song:
    return lrm.parse(
        build_container([build_region(*r) for r in regions]
                        + [build_event_stream([build_placement(*p) for p in placements])])
    )


# --------------------------------------------------------------------------- #
# the container walk
# --------------------------------------------------------------------------- #


def test_an_empty_container_has_no_regions_and_no_placements():
    song = lrm.parse(build_container([]))
    assert (song.regions, song.placements, song.event_stream_count) == ([], [], 0)


def test_bad_magic_is_a_typed_error():
    data = bytearray(build_container([]))
    data[0] = 0x00
    with pytest.raises(lrm.FormatError, match="bad magic"):
        lrm.parse(bytes(data))


def test_a_root_length_that_disagrees_with_the_file_is_refused():
    data = bytearray(build_container([build_event_stream([])]))
    struct.pack_into("<I", data, lrm.LENGTH_FIELD_OFFSET, 999_999)
    with pytest.raises(lrm.FormatError, match="root length mismatch"):
        lrm.parse(bytes(data))


def test_a_payload_size_past_eof_is_refused_rather_than_read():
    record = bytearray(build_record(lrm.TAG_EVENTS, b""))
    struct.pack_into("<I", record, lrm.PAYLOAD_SIZE_FIELD_OFFSET, 4096)
    data = build_container([bytes(record)])
    with pytest.raises(lrm.FormatError, match="past EOF"):
        lrm.parse(data)


def test_truncated_input_is_a_typed_error_not_a_crash():
    with pytest.raises(lrm.FormatError, match="too short"):
        lrm.parse(b"\x23\x47")


def test_a_record_header_truncated_at_eof_is_refused():
    # Six trailing bytes: too few for the 36-byte record header that must follow.
    data = bytearray(build_container([build_event_stream([])]) + b"gRuA\x00\x00")
    struct.pack_into(
        "<I", data, lrm.LENGTH_FIELD_OFFSET, len(data) - lrm.ROOT_HEADER_LEN
    )
    with pytest.raises(lrm.FormatError, match="runs past EOF"):
        lrm.parse(bytes(data))


def test_an_absurd_record_count_is_refused_rather_than_walked(monkeypatch):
    monkeypatch.setattr(lrm, "MAX_RECORDS", 1)
    data = build_container([build_event_stream([]), build_event_stream([])])
    with pytest.raises(lrm.FormatError, match="refusing to walk"):
        lrm.parse(data)


# --------------------------------------------------------------------------- #
# gRuA -- the region object
# --------------------------------------------------------------------------- #


def test_an_even_length_name_decodes_with_its_length_and_uuid():
    song = lrm.parse(build_container([build_region("Deep Down Shaker", 173509, uuid_bytes(0x2D))]))
    assert [(r.name, r.length_frames, r.uuid) for r in song.regions] == [
        ("Deep Down Shaker", 173509, "2d5ee052-641c-11f1-854a-205d0d40b51c")
    ]


def test_an_odd_length_name_is_padded_and_still_decodes():
    # The size law is 209 + nlen + (nlen & 1); an odd name adds a pad byte, and
    # everything behind the name -- including the UUID -- shifts with it.
    song = lrm.parse(build_container([build_region("Slow Drift Beat", 1676673, uuid_bytes(0x2D))]))
    assert [(r.name, r.length_frames, r.uuid) for r in song.regions] == [
        ("Slow Drift Beat", 1676673, "2d5ee052-641c-11f1-854a-205d0d40b51c")
    ]


def test_a_record_whose_size_breaks_the_law_is_skipped_not_guessed_at():
    record = bytearray(build_region("Deep Down Shaker", 173509, uuid_bytes(1)))
    record.append(0x00)  # one byte too long: every offset behind the name is now a guess
    struct.pack_into(
        "<I", record, lrm.PAYLOAD_SIZE_FIELD_OFFSET, len(record) - lrm.RECORD_HEADER_LEN
    )
    song = lrm.parse(build_container([bytes(record)]))
    assert song.regions == []


def test_a_record_that_fails_to_decode_is_still_counted_as_seen():
    # The original --scan counted only decoded regions, so its decode rate could
    # never fall below 100%. A failed record must show up in the denominator.
    broken = bytearray(build_region("Kick", 100, uuid_bytes(2)))
    struct.pack_into("<H", broken, lrm.REGION_NAME_LEN_OFFSET, 0)
    song = lrm.parse(
        build_container([build_region("Loop", 100, uuid_bytes(1)), bytes(broken)])
    )
    assert (len(song.regions), song.region_records_seen) == (1, 2)


def test_a_record_too_short_to_hold_a_name_length_is_skipped():
    song = lrm.parse(build_container([build_record(lrm.TAG_REGION, b"\x00" * 8)]))
    assert (song.regions, song.region_records_seen) == ([], 1)


def test_a_name_that_is_not_utf8_is_skipped():
    record = bytearray(build_region("Kick", 100, uuid_bytes(1)))
    record[lrm.REGION_NAME_OFFSET : lrm.REGION_NAME_OFFSET + 4] = b"\xff\xfe\xfd\xfc"
    song = lrm.parse(build_container([bytes(record)]))
    assert song.regions == []


def test_a_non_ascii_utf8_name_decodes_as_utf8():
    song = lrm.parse(build_container([build_region("Café", 100, uuid_bytes(1))]))
    assert [r.name for r in song.regions] == ["Café"]


def test_a_name_holding_a_control_character_is_skipped():
    record = bytearray(build_region("Kick", 100, uuid_bytes(1)))
    record[lrm.REGION_NAME_OFFSET + 1] = 0x07
    song = lrm.parse(build_container([bytes(record)]))
    assert song.regions == []


def test_a_zero_length_name_is_skipped():
    record = bytearray(build_region("Kick", 100, uuid_bytes(1)))
    struct.pack_into("<H", record, lrm.REGION_NAME_LEN_OFFSET, 0)
    song = lrm.parse(build_container([bytes(record)]))
    assert song.regions == []


def test_a_name_length_beyond_the_cap_is_skipped():
    record = bytearray(build_region("Kick", 100, uuid_bytes(1)))
    struct.pack_into("<H", record, lrm.REGION_NAME_LEN_OFFSET, lrm.MAX_REGION_NAME_LEN + 1)
    song = lrm.parse(build_container([bytes(record)]))
    assert song.regions == []


def test_regions_sharing_an_idx_form_one_family():
    song = lrm.parse(
        build_container(
            [
                build_region("Silky Acid Bass", 651323, uuid_bytes(1), family=7),
                build_region("Silky Acid Bass.1", 651323, uuid_bytes(2), family=7),
                build_region("Windmill Synth", 338688, uuid_bytes(3), family=9),
            ]
        )
    )
    assert {k: len(v) for k, v in song.families().items()} == {7: 2, 9: 1}


# --------------------------------------------------------------------------- #
# qSvE -- the 16-byte unit grid
# --------------------------------------------------------------------------- #


def test_a_payload_that_is_not_a_multiple_of_sixteen_is_refused_in_strict_mode():
    record = build_record(lrm.TAG_EVENTS, b"\x00" * 20)
    with pytest.raises(lrm.FormatError, match="not a multiple of 16"):
        lrm.parse(build_container([record]))


def test_a_misaligned_payload_is_counted_rather_than_raised_when_not_strict():
    record = build_record(lrm.TAG_EVENTS, b"\x00" * 20)
    song = lrm.parse(build_container([record]), strict=False)
    assert (song.event_streams_misaligned, song.placements) == (1, [])


def test_a_placement_decodes_to_its_track_bar_and_family():
    song = lrm.parse(build_container([build_event_stream([build_placement(7, 16.0, 2)])]))
    assert [(p.track, p.bar, p.family) for p in song.placements] == [(7, 16.0, 2)]


def test_a_half_bar_position_is_kept_not_rounded():
    song = lrm.parse(build_container([build_event_stream([build_placement(3, 55.5, 1)])]))
    assert [p.bar for p in song.placements] == [55.5]


def test_a_marker_carrying_track_zero_is_rejected_as_a_collision():
    song = lrm.parse(build_container([build_event_stream([build_placement(0, 5.0, 1)])]))
    assert (song.placements, dict(song.placement_rejects)) == ([], {"track_zero": 1})


def test_a_marker_positioned_beyond_any_real_song_is_rejected():
    event = bytearray(build_placement(4, 1.0, 1))
    struct.pack_into("<I", event, lrm.PLACEMENT_POSITION_OFFSET, 3_154_082_048)
    song = lrm.parse(build_container([build_event_stream([bytes(event)])]))
    assert (song.placements, dict(song.placement_rejects)) == ([], {"beyond_max_bar": 1})


def test_a_marker_before_the_region_time_origin_is_rejected_as_before_origin():
    # This is also where a genuine pre-roll placement would land: counted, not decoded.
    event = bytearray(build_placement(4, 1.0, 1))
    struct.pack_into("<I", event, lrm.PLACEMENT_POSITION_OFFSET, 12)
    song = lrm.parse(build_container([build_event_stream([bytes(event)])]))
    assert (song.placements, dict(song.placement_rejects)) == ([], {"before_origin": 1})


def test_a_group_truncated_by_the_end_of_the_payload_is_rejected():
    # A marker in the last 16-byte unit: the group's fields run past the payload.
    stream = build_record(lrm.TAG_EVENTS, lrm.PLACEMENT_MARKER + b"\x00" * 12)
    song = lrm.parse(build_container([stream]))
    assert (song.placements, dict(song.placement_rejects)) == ([], {"truncated": 1})
    assert song.placement_markers_rejected == 1


def test_a_group_whose_unit_types_differ_is_decoded_but_not_counted_as_known():
    event = bytearray(build_placement(4, 1.0, 1))
    event[0x20 + lrm.EVENT_TYPE_BYTE] = 0x88
    song = lrm.parse(build_container([build_event_stream([bytes(event)])]))
    assert (len(song.placements), song.placements_with_known_unit_types) == (1, 0)


def test_gaps_between_placement_heads_are_measured_in_bytes():
    filler = b"\x00" * 7 + b"\xaa" + b"\x00" * 8  # one non-placement unit
    stream = build_event_stream(
        [build_placement(1, 1.0, 1), filler * 2, build_placement(1, 5.0, 1)]
    )
    song = lrm.parse(build_container([stream]))
    assert dict(song.placement_head_gaps) == {48 + 32: 1}


def test_a_stream_without_its_terminator_is_counted():
    stream = build_record(lrm.TAG_EVENTS, build_placement(1, 1.0, 1))
    song = lrm.parse(build_container([stream]))
    assert (
        song.event_streams_with_one_terminator,
        song.event_streams_ending_in_terminator,
    ) == (0, 0)


def test_placements_are_returned_in_timeline_order():
    song = lrm.parse(
        build_container(
            [
                build_event_stream(
                    [
                        build_placement(1, 21.0, 1),
                        build_placement(2, 5.0, 2),
                        build_placement(3, 16.0, 3),
                    ]
                )
            ]
        )
    )
    assert [p.bar for p in song.placements] == [5.0, 16.0, 21.0]


# --------------------------------------------------------------------------- #
# joining placements to regions
# --------------------------------------------------------------------------- #


def test_a_single_region_family_names_its_placement_exactly():
    song = song_with([("Slow Drift Beat", 1676673, uuid_bytes(1), 5)], [(22, 17.0, 5)])
    assert lrm.family_label(song, 5) == "Slow Drift Beat"


def test_a_multi_copy_family_reports_the_stem_and_the_count():
    song = song_with(
        [
            ("Angelic Vocal FX 03", 156145, uuid_bytes(1), 8),
            ("Angelic Vocal FX 03.1", 156145, uuid_bytes(2), 8),
        ],
        [(13, 1.0, 8)],
    )
    assert lrm.family_label(song, 8) == "Angelic Vocal FX 03 (one of 2 copies)"


def test_the_copy_count_is_region_objects_not_distinct_names():
    song = song_with(
        [("Loop", 1, uuid_bytes(1), 8), ("Loop", 1, uuid_bytes(2), 8)], [(1, 1.0, 8)]
    )
    assert lrm.family_label(song, 8) == "Loop (one of 2 copies)"


def test_the_stem_tie_break_does_not_depend_on_hash_order():
    # Two names of equal length: the stem must be the same whatever order the
    # records arrive in, or the Rust port cannot reproduce it.
    names = ("Beta", "Alfa")
    for order in (names, names[::-1]):
        song = song_with(
            [(n, 1, uuid_bytes(i), 3) for i, n in enumerate(order)], [(1, 1.0, 3)]
        )
        assert lrm.family_name(song, 3) == ("Alfa", 2)


def test_a_placement_linking_to_no_region_says_so_rather_than_inventing_a_name():
    song = song_with([], [(1, 1.0, 4)])
    assert lrm.family_label(song, 4) == "<unresolved family 4>"


def test_the_surplus_is_a_count_per_family_and_names_no_copy():
    song = song_with(
        [
            ("Deep Down Shaker", 173509, uuid_bytes(1), 0),
            ("Deep Down Shaker.1", 173509, uuid_bytes(2), 0),
            ("Deep Down Shaker.2", 173509, uuid_bytes(3), 0),
            ("Loop", 1, uuid_bytes(4), 1),
        ],
        [(1, 1.0, 0), (1, 5.0, 1)],
    )
    assert lrm.family_surplus(song) == [("Deep Down Shaker", 3, 1)]


# --------------------------------------------------------------------------- #
# bars -- exact, never rounded
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize(
    "tick,text",
    [
        (0, "1"),
        (8 * 3840, "9"),
        (54 * 3840 + 1920, "55.5"),
        (54 * 3840 + 2880, "55.75"),
        (65 * 3840 + 3200, "66 5/6"),
        (99 * 3840 + 1, "100 1/3840"),
    ],
)
def test_format_bar_is_exact(tick, text):
    assert lrm.format_bar(lrm.REGION_TIME_ORIGIN + tick) == text


def test_a_one_tick_move_is_visible_in_the_diff():
    regions = [("Loop", 1, uuid_bytes(1), 2)]
    old = song_with(regions, [])
    new = song_with(regions, [])
    old.placements = [lrm.Placement(1, lrm.REGION_TIME_ORIGIN + 99 * 3840, 2, 0)]
    new.placements = [lrm.Placement(1, lrm.REGION_TIME_ORIGIN + 99 * 3840 + 1, 2, 0)]
    assert lrm.report_diff(old, new) == [
        "  track 1: 'Loop' moved from bar 100 to bar 100 1/3840"
    ]


# --------------------------------------------------------------------------- #
# the diff -- regions keyed on the UUID, placements on (track, position, family)
# --------------------------------------------------------------------------- #

KICK = ("Kick", 1000, uuid_bytes(1), 1)
SNARE = ("Snare", 1000, uuid_bytes(2), 2)
CLAP = ("Clap", 1000, uuid_bytes(3), 3)


def test_a_region_moving_one_bar_is_the_only_thing_reported():
    old = song_with([("Deep Down Shaker", 173509, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    new = song_with([("Deep Down Shaker", 173509, uuid_bytes(1), 2)], [(1, 5.0, 2)])
    assert lrm.report_diff(old, new) == [
        "  track 1: 'Deep Down Shaker' moved from bar 1 to bar 5"
    ]


def test_moving_one_region_past_another_reports_only_that_move():
    # The original diff paired placements by bar order per track and reported
    # "Kick moved 1->5" and "Snare moved 5->9" here. Only the Kick moved.
    old = song_with([KICK, SNARE], [(1, 1.0, 1), (1, 5.0, 2)])
    new = song_with([KICK, SNARE], [(1, 5.0, 2), (1, 9.0, 1)])
    assert lrm.report_diff(old, new) == ["  track 1: 'Kick' moved from bar 1 to bar 9"]


def test_a_region_replaced_by_another_family_is_removed_plus_added_not_moved():
    old = song_with([SNARE, CLAP], [(1, 5.0, 2)])
    new = song_with([SNARE, CLAP], [(1, 9.0, 3)])
    assert lrm.report_diff(old, new) == [
        "  track 1: placement removed: 'Snare' at bar 5",
        "  track 1: placement added: 'Clap' at bar 9",
    ]


def test_two_regions_swapping_positions_are_both_reported():
    # Same bar set before and after: the original diff compared positions only
    # and reported no change.
    old = song_with([KICK, SNARE], [(1, 1.0, 1), (1, 5.0, 2)])
    new = song_with([KICK, SNARE], [(1, 1.0, 2), (1, 5.0, 1)])
    assert lrm.report_diff(old, new) == [
        "  track 1: 'Kick' moved from bar 1 to bar 5",
        "  track 1: 'Snare' moved from bar 5 to bar 1",
    ]


def test_a_region_replaced_at_the_same_bar_is_reported():
    old = song_with([KICK, SNARE], [(1, 5.0, 1)])
    new = song_with([KICK, SNARE], [(1, 5.0, 2)])
    assert lrm.report_diff(old, new) == [
        "  track 1: placement removed: 'Kick' at bar 5",
        "  track 1: placement added: 'Snare' at bar 5",
    ]


def test_a_move_to_another_track_names_both_tracks():
    old = song_with([KICK], [(1, 5.0, 1)])
    new = song_with([KICK], [(2, 5.0, 1)])
    assert lrm.report_diff(old, new) == [
        "  'Kick' moved from track 1 bar 5 to track 2 bar 5"
    ]


def test_an_ambiguous_same_family_pairing_is_not_guessed():
    # Two placements of one family both left, two new ones appeared: which went
    # where is not decodable, so nothing is called a move.
    old = song_with([KICK], [(1, 1.0, 1), (1, 5.0, 1)])
    new = song_with([KICK], [(1, 9.0, 1), (1, 13.0, 1)])
    assert lrm.report_diff(old, new) == [
        "  track 1: placement removed: 'Kick' at bar 1",
        "  track 1: placement removed: 'Kick' at bar 5",
        "  track 1: placement added: 'Kick' at bar 9",
        "  track 1: placement added: 'Kick' at bar 13",
    ]


def test_a_rename_is_seen_because_the_uuid_did_not_change():
    old = song_with([("Take 1", 173509, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    new = song_with([("Chorus", 173509, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    assert lrm.report_diff(old, new) == ["  region renamed: 'Take 1' -> 'Chorus'"]


def test_a_trim_is_reported_in_frames():
    old = song_with([("Loop", 235200, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    new = song_with([("Loop", 89128, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    assert lrm.report_diff(old, new) == ["  region 'Loop' resized: 235200 -> 89128 frames"]


def test_an_added_region_is_reported_by_name():
    old = song_with([("Loop", 235200, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    new = song_with(
        [("Loop", 235200, uuid_bytes(1), 2), ("Loop.1", 235200, uuid_bytes(2), 3)],
        [(1, 1.0, 2)],
    )
    assert lrm.report_diff(old, new) == ["  region added: 'Loop.1'"]


def test_a_removed_region_is_reported_by_name():
    old = song_with(
        [("Loop", 235200, uuid_bytes(1), 2), ("Loop.1", 235200, uuid_bytes(2), 3)],
        [(1, 1.0, 2)],
    )
    new = song_with([("Loop", 235200, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    assert lrm.report_diff(old, new) == ["  region removed: 'Loop.1'"]


def test_a_move_within_a_multi_copy_family_does_not_claim_which_copy():
    regions = [
        ("Silky Acid Bass", 651323, uuid_bytes(1), 8),
        ("Silky Acid Bass.1", 651323, uuid_bytes(2), 8),
    ]
    old = song_with(regions, [(4, 1.0, 8)])
    new = song_with(regions, [(4, 9.0, 8)])
    assert lrm.report_diff(old, new) == [
        "  track 4: a 'Silky Acid Bass' region moved from bar 1 to bar 9"
    ]


def test_an_added_placement_is_reported_without_pairing_guesses():
    regions = [("Loop", 235200, uuid_bytes(1), 2)]
    old = song_with(regions, [(3, 1.0, 2)])
    new = song_with(regions, [(3, 1.0, 2), (3, 9.0, 2)])
    assert lrm.report_diff(old, new) == ["  track 3: placement added: 'Loop' at bar 9"]


def test_only_the_placement_that_moved_is_reported():
    # Two placements of one family on one track; the first stays, the second
    # moves. The unmoved one cancels out and must not appear.
    regions = [("Loop", 235200, uuid_bytes(1), 2)]
    old = song_with(regions, [(1, 1.0, 2), (1, 5.0, 2)])
    new = song_with(regions, [(1, 1.0, 2), (1, 9.0, 2)])
    assert lrm.report_diff(old, new) == [
        "  track 1: 'Loop' moved from bar 5 to bar 9"
    ]


def test_two_identical_saves_report_no_change():
    song = song_with([("Loop", 235200, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    assert lrm.report_diff(song, song) == ["  no region or placement change"]


# --------------------------------------------------------------------------- #
# input resolution
# --------------------------------------------------------------------------- #


def test_a_bare_project_data_file_resolves(tmp_path):
    path = tmp_path / "ProjectData"
    path.write_bytes(build_container([]))
    assert lrm.resolve_project_data(path) == path


def test_a_bundle_resolves_through_alternatives(tmp_path):
    inner = tmp_path / "P.logicx" / "Alternatives" / "000"
    inner.mkdir(parents=True)
    (inner / "ProjectData").write_bytes(build_container([]))
    assert lrm.resolve_project_data(tmp_path / "P.logicx") == inner / "ProjectData"


def test_a_garageband_xml_project_data_does_not_shadow_the_real_container(tmp_path):
    # A .band ships a legacy XML `projectData` at its top level. On macOS's
    # case-insensitive filesystem that matches a `ProjectData` lookup, so the
    # candidate must be checked for the container magic, not taken on name.
    bundle = tmp_path / "P.band"
    inner = bundle / "Alternatives" / "000"
    inner.mkdir(parents=True)
    (bundle / "ProjectData").write_bytes(b'<?xml version="1.0"?><plist/>')
    (inner / "ProjectData").write_bytes(build_container([]))
    assert lrm.resolve_project_data(bundle) == inner / "ProjectData"


def test_a_path_that_does_not_exist_is_a_typed_error(tmp_path):
    with pytest.raises(lrm.FormatError, match="no ProjectData found"):
        lrm.resolve_project_data(tmp_path / "nothing here")


def test_a_bundle_with_no_current_save_still_finds_its_backups(tmp_path):
    bundle = make_bundle(tmp_path, "P.logicx", {"00": build_container([])})
    assert [p.parent.name for p in lrm.saves_in(bundle)] == ["00"]


def test_a_directory_with_no_container_is_a_typed_error(tmp_path):
    with pytest.raises(lrm.FormatError, match="no ProjectData found"):
        lrm.resolve_project_data(tmp_path)


def test_a_present_but_unreadable_candidate_says_what_it_found(tmp_path):
    (tmp_path / "ProjectData").write_bytes(b"not a container")
    with pytest.raises(lrm.FormatError, match="not a ProjectData container"):
        lrm.resolve_project_data(tmp_path)


# --------------------------------------------------------------------------- #
# cli
# --------------------------------------------------------------------------- #


def test_the_cli_requires_exactly_one_mode():
    with pytest.raises(SystemExit):
        lrm.main([])


def test_the_map_counts_surplus_region_objects_without_naming_copies(tmp_path, capsys):
    path = tmp_path / "ProjectData"
    path.write_bytes(
        build_container(
            [
                build_region("Deep Down Shaker", 173509, uuid_bytes(1), family=0),
                build_region("Deep Down Shaker.1", 173509, uuid_bytes(2), family=0),
                build_event_stream([build_placement(1, 1.0, 0)]),
            ]
        )
    )
    assert lrm.main([str(path)]) == 0
    out = capsys.readouterr().out
    assert "FAMILIES WITH MORE REGION OBJECTS THAN PLACEMENTS (1)" in out
    assert "Deep Down Shaker                   2 region object(s), 1 placed: 1 not on the timeline" in out
    assert "Deep Down Shaker.1" not in out


def test_the_map_says_how_many_placements_cannot_be_attributed_to_a_copy(tmp_path, capsys):
    path = tmp_path / "ProjectData"
    path.write_bytes(
        build_container(
            [
                build_region("Loop", 1, uuid_bytes(1), family=1),
                build_region("Pad", 1, uuid_bytes(2), family=2),
                build_region("Pad.1", 1, uuid_bytes(3), family=2),
                build_event_stream(
                    [build_placement(1, 1.0, 1), build_placement(2, 1.0, 2), build_placement(2, 5.0, 2)]
                ),
            ]
        )
    )
    assert lrm.main([str(path)]) == 0
    out = capsys.readouterr().out
    assert "1 of 3 placements are in a family holding one region object" in out
    assert "2 of 3 placements are in the 1 of 2 families holding 2+ region objects" in out
    assert "byte +7 values (4 distinct): 00:3 3f:1 89:3 bc:3" in out
    assert "carry byte +7 = 00/89/bc: 3 of 3" in out


def test_the_cli_reports_a_map(tmp_path, capsys):
    path = tmp_path / "ProjectData"
    path.write_bytes(
        build_container(
            [
                build_region("Deep Down Shaker", 173509, uuid_bytes(1), family=2),
                build_event_stream([build_placement(7, 12.0, 2)]),
            ]
        )
    )
    assert lrm.main([str(path)]) == 0
    out = capsys.readouterr().out
    assert "track 7   bar 12         Deep Down Shaker" in out
    assert "1 of 1 region records decoded, in 1 families" in out


def test_the_cli_reports_a_diff_as_the_sentence_the_issue_asks_for(tmp_path, capsys):
    def save(name: str, bar: float) -> str:
        path = tmp_path / name
        path.write_bytes(
            build_container(
                [
                    build_region("Deep Down Shaker.1", 173509, uuid_bytes(1), family=2),
                    build_event_stream([build_placement(7, bar, 2)]),
                ]
            )
        )
        return str(path)

    assert lrm.main(["--diff", save("a", 12.0), save("b", 16.0)]) == 0
    assert capsys.readouterr().out == (
        "  track 7: 'Deep Down Shaker.1' moved from bar 12 to bar 16\n"
    )


def test_the_cli_exits_nonzero_on_unreadable_input(tmp_path, capsys):
    path = tmp_path / "ProjectData"
    path.write_bytes(b"nope")
    assert lrm.main([str(path)]) == 1
    assert "not readable" in capsys.readouterr().err


def test_the_cli_emits_json_when_asked(tmp_path, capsys):
    import json

    path = tmp_path / "ProjectData"
    path.write_bytes(
        build_container(
            [
                build_region("Loop", 235200, uuid_bytes(1), family=2),
                build_event_stream([build_placement(3, 5.0, 2)]),
            ]
        )
    )
    assert lrm.main([str(path), "--json"]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["regions"][0]["name"] == "Loop"
    assert payload["placements"][0]["bar"] == 5.0
    assert payload["region_records_seen"] == 1


# --------------------------------------------------------------------------- #
# --scan -- library-wide decode rates
# --------------------------------------------------------------------------- #


def make_bundle(root, name: str, saves: dict[str, bytes], tracks: int | None = None):
    """A .logicx-shaped bundle: a current save plus numbered backup slots."""
    import plistlib

    alternative = root / name / "Alternatives" / "000"
    alternative.mkdir(parents=True)
    for slot, data in saves.items():
        target = alternative if slot == "current" else (
            alternative / "Project File Backups" / slot
        )
        target.mkdir(parents=True, exist_ok=True)
        (target / "ProjectData").write_bytes(data)
        if tracks is not None:
            with (target / "MetaData.plist").open("wb") as handle:
                plistlib.dump({"NumberOfTracks": tracks}, handle)
    return root / name


def test_saves_in_finds_the_current_save_and_every_backup_slot(tmp_path):
    bundle = make_bundle(
        tmp_path,
        "P.logicx",
        {"current": build_container([]), "00": build_container([]), "01": build_container([])},
    )
    assert len(lrm.saves_in(bundle)) == 3


def test_declared_track_count_reads_the_sibling_metadata_plist(tmp_path):
    bundle = make_bundle(tmp_path, "P.logicx", {"current": build_container([])}, tracks=31)
    assert lrm.declared_track_count(lrm.saves_in(bundle)[0]) == 31


def test_declared_track_count_is_none_when_there_is_no_plist(tmp_path):
    bundle = make_bundle(tmp_path, "P.logicx", {"current": build_container([])})
    assert lrm.declared_track_count(lrm.saves_in(bundle)[0]) is None


def test_declared_track_count_is_none_when_the_plist_is_corrupt(tmp_path):
    bundle = make_bundle(tmp_path, "P.logicx", {"current": build_container([])}, tracks=31)
    save = lrm.saves_in(bundle)[0]
    (save.parent / "MetaData.plist").write_bytes(b"not a plist")
    assert lrm.declared_track_count(save) is None


def test_scan_counts_regions_placements_and_event_streams(tmp_path):
    data = build_container(
        [
            build_region("Loop", 235200, uuid_bytes(1), family=2),
            build_event_stream([build_placement(3, 5.0, 2)]),
        ]
    )
    make_bundle(tmp_path, "P.logicx", {"current": data, "00": data}, tracks=4)
    totals = lrm.scan_library(tmp_path)
    assert (
        totals["bundles"],
        totals["saves"],
        totals["saves_walked"],
        totals["region_records_seen"],
        totals["region_records_decoded"],
        totals["placements"],
        totals["placements_resolved_to_a_region"],
        totals["placements_on_the_960_grid"],
        totals["placements_with_unit_types_00_89_bc"],
        totals["placement_highest_bar"],
        totals["event_streams_with_exactly_one_terminator"],
        totals["event_streams_ending_in_terminator"],
        totals["saves_track_count_within_declared"],
    ) == (1, 2, 2, 2, 2, 2, 2, 2, 2, 5, 2, 2, 2)


def test_scan_decode_rate_can_fall_below_one_hundred_percent(tmp_path):
    broken = bytearray(build_region("Kick", 100, uuid_bytes(2)))
    broken.append(0)  # breaks the size law
    struct.pack_into(
        "<I", broken, lrm.PAYLOAD_SIZE_FIELD_OFFSET, len(broken) - lrm.RECORD_HEADER_LEN
    )
    data = build_container([build_region("Loop", 100, uuid_bytes(1)), bytes(broken)])
    make_bundle(tmp_path, "P.logicx", {"current": data})
    totals = lrm.scan_library(tmp_path)
    assert (totals["region_records_decoded"], totals["region_records_seen"]) == (1, 2)


def test_scan_breaks_rejects_down_by_reason_and_prints_histograms(tmp_path):
    beyond = bytearray(build_placement(4, 1.0, 1))
    struct.pack_into("<I", beyond, lrm.PLACEMENT_POSITION_OFFSET, 3_154_082_048)
    data = build_container(
        [build_event_stream([build_placement(0, 5.0, 1), bytes(beyond), build_placement(2, 3.0, 1)])]
    )
    make_bundle(tmp_path, "P.logicx", {"current": data})
    totals = lrm.scan_library(tmp_path)
    assert (
        totals["placement_markers_seen"],
        totals["placement_markers_rejected"],
        totals["placement_markers_rejected_track_zero"],
        totals["placement_markers_rejected_beyond_max_bar"],
        totals["placement_markers_rejected_before_origin"],
        totals["placement_markers_rejected_truncated"],
    ) == (3, 2, 1, 1, 0, 0)
    # The out-of-range position's high byte IS the head unit's byte +7 (0xbb
    # here), which is why a nonzero head type byte flags a collision.
    assert totals["event_type_byte"] == {
        "0x00": 2, "0x3f": 1, "0x89": 3, "0xbb": 1, "0xbc": 3
    }
    assert totals["placement_head_gap_bytes"] == {}


def test_scan_counts_an_unreadable_save_without_aborting_the_library(tmp_path):
    make_bundle(tmp_path, "P.logicx", {"current": b"not a container"})
    totals = lrm.scan_library(tmp_path)
    assert (totals["saves"], totals["saves_walked"], totals["saves_unreadable"]) == (1, 0, 1)


def test_scan_counts_a_misaligned_event_stream_instead_of_failing_the_save(tmp_path):
    data = build_container([build_record(lrm.TAG_EVENTS, b"\x00" * 20)])
    make_bundle(tmp_path, "P.logicx", {"current": data})
    totals = lrm.scan_library(tmp_path)
    assert (totals["saves_walked"], totals["event_streams_misaligned"]) == (1, 1)


def test_scan_flags_a_save_whose_track_number_exceeds_its_declared_count(tmp_path):
    data = build_container(
        [
            build_region("Loop", 235200, uuid_bytes(1), family=2),
            build_event_stream([build_placement(9, 5.0, 2)]),
        ]
    )
    make_bundle(tmp_path, "P.logicx", {"current": data}, tracks=5)
    totals = lrm.scan_library(tmp_path)
    assert (
        totals["saves_with_declared_track_count"],
        totals["saves_track_count_within_declared"],
    ) == (1, 0)


def test_scan_ignores_a_directory_that_is_not_a_bundle(tmp_path):
    (tmp_path / "notes.logicx").mkdir()
    assert lrm.scan_library(tmp_path)["bundles"] == 0


def test_the_cli_scans_a_library(tmp_path, capsys):
    make_bundle(tmp_path, "P.logicx", {"current": build_container([])})
    assert lrm.main(["--scan", str(tmp_path)]) == 0
    out = capsys.readouterr().out
    assert re.search(r"^bundles +1$", out, re.M)
    assert "event_type_byte (0 distinct)" in out


def test_the_cli_scans_a_library_as_json(tmp_path, capsys):
    import json

    make_bundle(tmp_path, "P.logicx", {"current": build_container([])})
    assert lrm.main(["--scan", str(tmp_path), "--json"]) == 0
    assert json.loads(capsys.readouterr().out)["bundles"] == 1


def test_the_cli_emits_a_diff_as_json(tmp_path, capsys):
    import json

    def save(name: str, bar: float) -> str:
        path = tmp_path / name
        path.write_bytes(
            build_container(
                [
                    build_region("Loop", 235200, uuid_bytes(1), family=2),
                    build_event_stream([build_placement(1, bar, 2)]),
                ]
            )
        )
        return str(path)

    assert lrm.main(["--diff", save("a", 1.0), save("b", 3.0), "--json"]) == 0
    assert json.loads(capsys.readouterr().out)["changes"] == [
        "track 1: 'Loop' moved from bar 1 to bar 3"
    ]


def test_the_cli_rejects_two_modes_at_once(tmp_path):
    with pytest.raises(SystemExit):
        lrm.main([str(tmp_path), "--scan", str(tmp_path)])


# --------------------------------------------------------------------------- #
# real material -- loud skip when WIT_FIXTURES is unset
# --------------------------------------------------------------------------- #


@pytest.mark.real_fixtures
def test_real_saves_walk_and_every_event_stream_is_sixteen_byte_aligned(real_material):
    for path in real_material.logic_project_data(min_len=2)[:10]:
        song = lrm.parse(path.read_bytes())
        assert song.event_streams_misaligned == 0


@pytest.mark.real_fixtures
def test_every_real_region_record_seen_decodes_with_a_distinct_uuid(real_material):
    for path in real_material.logic_project_data(min_len=2)[:10]:
        song = lrm.parse(path.read_bytes())
        assert len(song.regions) == song.region_records_seen
        assert len({r.uuid for r in song.regions}) == len(song.regions)


@pytest.mark.real_fixtures
def test_every_real_placement_resolves_to_a_region_family(real_material):
    for path in real_material.logic_project_data(min_len=2)[:10]:
        song = lrm.parse(path.read_bytes())
        if not song.regions:
            continue
        families = song.families()
        assert all(p.family in families for p in song.placements)
