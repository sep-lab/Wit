"""
The fixture factories are load-bearing: every other test in this suite trusts
them, so they are verified by round-tripping through the real prototypes.

If a test here fails, do not trust any other failure in the suite until it is fixed.
"""

from __future__ import annotations

import gzip
import struct
import xml.etree.ElementTree as ET

import pytest
from factories import binary
from factories.als import LiveSet, Track, als_bytes, simple_set
from factories.flp import (
    FlpBuilder,
    decode_varint,
    encode_varint,
    latin1_text,
    realistic_project,
    utf16_text,
)

# --------------------------------------------------------------------------- #
# .als factory
# --------------------------------------------------------------------------- #


def test_als_is_gzipped_xml_with_ableton_root(write_als):
    path = write_als(simple_set())
    with open(path, "rb") as fh:
        assert fh.read(2) == b"\x1f\x8b", "an .als is gzip; the magic must be right"
    with gzip.open(path, "rb") as fh:
        root = ET.parse(fh).getroot()
    assert root.tag == "Ableton"
    assert root.find("LiveSet") is not None


def test_als_round_trips_through_the_real_model_builder(model_of):
    """Everything the description declares must come back out of build_model."""
    described = simple_set()
    model = model_of(described)

    assert model["creator"] == described.creator
    assert model["tempo"] == 120.0
    assert set(model["tracks"]) == {"100", "101", "102"}

    drums = model["tracks"]["100"]
    assert drums["name"] == "Drums"
    assert drums["kind"] == "AudioTrack"
    assert drums["color"] == "13"
    assert drums["volume"] == 1.0
    assert drums["pan"] == 0.0
    assert drums["speaker"] == "true"
    assert drums["devices"] == ["Eq8"]
    assert set(drums["clips"]) == {"10", "11"}
    assert drums["clips"]["10"] == {
        "name": "kick loop",
        "start": 0.0,
        "end": 16.0,
        "sample": "kick.wav",  # the model keeps the basename only
        "disabled": "false",
    }

    bass = model["tracks"]["101"]
    assert bass["volume"] == 0.794
    assert bass["pan"] == -0.25
    assert bass["devices"] == []

    keys = model["tracks"]["102"]
    assert keys["kind"] == "MidiTrack"
    assert keys["automation_lanes"] == 1
    assert keys["notes"] == 12
    assert keys["clips"]["30"]["sample"] == ""  # MIDI clips have no sample


def test_als_sample_index_is_built_per_basename(model_of):
    model = model_of(simple_set())
    assert model["samples"]["kick.wav"] == {"Drums"}
    assert model["samples"]["sub.wav"] == {"Bass"}
    assert "chords" not in model["samples"]


def test_als_bytes_are_deterministic():
    """Same description -> same bytes. Tests may compare files, not just models."""
    assert als_bytes(simple_set()) == als_bytes(simple_set())


def test_als_copy_is_deep(model_of):
    original = simple_set()
    edited = original.copy()
    edited.track("Drums").volume = 0.5
    edited.track("Drums").clip("10").start = 4.0
    assert original.track("Drums").volume == 1.0
    assert original.track("Drums").clip("10").start == 0.0


@pytest.mark.parametrize("kind", ["AudioTrack", "MidiTrack", "GroupTrack", "ReturnTrack"])
def test_als_supports_every_track_kind_the_model_recognises(model_of, kind, als_diff):
    assert kind in als_diff.TRACK_TAGS
    ls = LiveSet(tracks=[Track(id="7", name="T", kind=kind)])
    model = model_of(ls)
    assert model["tracks"]["7"]["kind"] == kind


def test_als_factory_rejects_an_unknown_track_kind():
    with pytest.raises(ValueError, match="unknown track kind"):
        LiveSet(tracks=[Track(id="1", name="x", kind="GuitarTrack")]).to_xml_bytes()


def test_als_view_state_is_present_and_visible_in_the_raw_xml():
    """
    A "view state only" diff test is worthless if the fixture has no view state.
    """
    a = simple_set()
    b = a.copy()
    b.scroll_x = 4096
    b.zoom = 3.5
    b.selected_track = "101"
    b.tracks[0].selected = True
    b.tracks[0].clips[0].scroller_time = 12.5

    assert a.to_xml_bytes() != b.to_xml_bytes(), "the view change must reach the XML"


def test_als_special_characters_are_escaped(model_of):
    """Track names come from users; & and < must survive a round trip."""
    ls = LiveSet(tracks=[Track(id="1", name='Bass & "Sub" <live>')])
    assert model_of(ls)["tracks"]["1"]["name"] == 'Bass & "Sub" <live>'


# --------------------------------------------------------------------------- #
# .flp factory
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize(
    "value,expected",
    [
        (0, b"\x00"),
        (1, b"\x01"),
        (127, b"\x7f"),
        (128, b"\x80\x01"),
        (16383, b"\xff\x7f"),
        (16384, b"\x80\x80\x01"),
        (2097151, b"\xff\xff\x7f"),
        (2097152, b"\x80\x80\x80\x01"),
    ],
)
def test_varint_encoding_is_correct_at_every_width_boundary(value, expected):
    assert encode_varint(value) == expected
    assert decode_varint(expected) == (value, len(expected))


def test_varint_rejects_negative():
    with pytest.raises(ValueError):
        encode_varint(-1)


def test_flp_header_is_byte_exact():
    data = FlpBuilder(fmt=0, channels=18, ppq=96).to_bytes()
    assert data[:4] == b"FLhd"
    assert struct.unpack("<I", data[4:8])[0] == 6
    assert struct.unpack("<hHH", data[8:14]) == (0, 18, 96)
    assert data[14:18] == b"FLdt"
    assert struct.unpack("<I", data[18:22])[0] == 0


def test_flp_event_widths_are_implied_by_the_id_not_written_down():
    b = FlpBuilder().byte(0, 0xFF).word(64, 0xBEEF).dword(128, 0xDEADBEEF)
    stream = b.event_bytes
    assert stream == bytes([0, 0xFF]) + bytes([64, 0xEF, 0xBE]) + bytes(
        [128, 0xEF, 0xBE, 0xAD, 0xDE]
    )


@pytest.mark.parametrize("bad", [(0, 64), (64, 0), (64, 128), (128, 64), (128, 192)])
def test_flp_factory_refuses_ids_outside_their_width_class(bad):
    method, ev = ("byte", bad[1]) if bad[0] == 0 else (
        "word" if bad[0] == 64 else "dword",
        bad[1],
    )
    with pytest.raises(ValueError):
        getattr(FlpBuilder(), method)(ev, 0)


def test_flp_round_trips_through_the_real_parser(flp_report):
    built = realistic_project()
    report = flp_report(built.to_bytes())

    assert (report["format"], report["channels"], report["ppq"]) == (0, 2, 96)
    assert report["events"] == built.event_count
    assert report["distinct_ids"] == len({ev for ev, _ in built.log})

    # every event id the builder wrote appears with the right count
    from collections import Counter

    expected = Counter(ev for ev, _ in built.log)
    for ev, count in expected.items():
        assert report["rows"][ev]["count"] == count, "id %d miscounted" % ev

    # variable-length payload accounting
    expected_blob = sum(len(p) for ev, p in built.log if ev >= 192)
    assert report["blob_bytes"] == expected_blob
    assert report["file_bytes"] == len(built.to_bytes())


def test_flp_declared_data_length_matches_the_event_stream():
    built = realistic_project()
    data = built.to_bytes()
    pos = 8 + struct.unpack("<I", data[4:8])[0]
    assert data[pos:pos + 4] == b"FLdt"
    dlen = struct.unpack("<I", data[pos + 4:pos + 8])[0]
    assert dlen == len(built.event_bytes)
    assert pos + 8 + dlen == len(data), "no trailing slack after the event stream"


def test_flp_text_payloads_are_nul_terminated():
    assert latin1_text("Kick") == b"Kick\x00"
    assert utf16_text("Kick") == b"K\x00i\x00c\x00k\x00\x00\x00"


# --------------------------------------------------------------------------- #
# byte-stream factory
# --------------------------------------------------------------------------- #


def test_binary_streams_are_reproducible():
    assert binary.incompressible(4096, seed=3) == binary.incompressible(4096, seed=3)
    assert binary.incompressible(4096, seed=3) != binary.incompressible(4096, seed=4)
    assert binary.xmlish(20, seed=1) == binary.xmlish(20, seed=1)


def test_repeated_blocks_really_repeat():
    data = binary.repeated_blocks(1024, 5, seed=2)
    assert len(data) == 5120
    assert data[:1024] == data[1024:2048] == data[4096:]


def test_edit_helpers_do_what_they_say():
    data = b"0123456789"
    assert binary.insert_at(data, 3, b"XY") == b"012XY3456789"
    assert binary.replace_region(data, 3, b"XY") == b"012XY56789"
    assert binary.delete_region(data, 3, 2) == b"01256789"
    assert len(binary.replace_region(data, 0, b"AB")) == len(data)
