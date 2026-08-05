"""
FL Studio .flp event stream: sizing, varints, and text decoding.

docs/FORMATS.md and EXPERIMENTS.md 10 rest on this parser: "1,491 events, 82
distinct IDs, 92% of the file is opaque variable-length plugin state". Those
numbers are only meaningful if the event walk is exactly right — one mis-sized
event desynchronises the stream and everything after it is garbage that still
looks like plausible output.

The varint decoder is the sharpest edge: it is the only place where the parser
reads a length from the file itself.
"""

from __future__ import annotations

import pytest
from factories.flp import (
    FlpBuilder,
    encode_varint,
    latin1_text,
    realistic_project,
    utf16_text,
)

# --------------------------------------------------------------------------- #
# header
# --------------------------------------------------------------------------- #


def test_header_fields_are_reported(flp_report):
    data = FlpBuilder(fmt=0, channels=18, ppq=96).byte(0, 1).to_bytes()
    r = flp_report(data)
    assert (r["format"], r["channels"], r["ppq"]) == (0, 18, 96)


def test_format_field_is_signed(flp_report):
    """`struct.unpack("<hHH")` — format is i16, so -1 must not read as 65535."""
    r = flp_report(FlpBuilder(fmt=-1).byte(0, 0).to_bytes())
    assert r["format"] == -1


def test_empty_event_stream_is_not_an_error(flp_report):
    r = flp_report(FlpBuilder().to_bytes())
    assert r["data_length"] == 0
    assert r["events"] == 0


# --------------------------------------------------------------------------- #
# the four width classes
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize(
    "ev,method,expected_payload",
    [
        (0, "byte", 1),
        (63, "byte", 1),
        (64, "word", 2),
        (127, "word", 2),
        (128, "dword", 4),
        (191, "dword", 4),
    ],
)
def test_fixed_width_classes_consume_exactly_their_width(
    flp_report, ev, method, expected_payload
):
    """
    Boundary ids (63/64, 127/128, 191/192) are where an off-by-one in the class
    test would desynchronise the whole stream.
    """
    b = FlpBuilder()
    getattr(b, method)(ev, 1)
    b.byte(1, 7)  # a sentinel that only parses if the previous event was sized right
    r = flp_report(b.to_bytes())
    assert r["events"] == 2
    assert r["rows"][ev]["count"] == 1
    assert r["rows"][1]["count"] == 1
    # fixed-width payloads are not counted as variable-length payload
    assert r["blob_bytes"] == 0


@pytest.mark.parametrize("ev", [192, 209, 255])
def test_variable_width_ids_read_a_length_prefix(flp_report, ev):
    b = FlpBuilder().var(ev, b"\x01" * 300).byte(1, 7)
    r = flp_report(b.to_bytes())
    assert r["events"] == 2
    assert r["rows"][ev]["payload_bytes"] == 300


def test_all_four_classes_interleave_without_desynchronising(flp_report):
    b = (
        FlpBuilder()
        .byte(5, 1)
        .var(210, b"x" * 200)
        .word(70, 2)
        .var(211, b"y" * 5)
        .dword(150, 3)
        .byte(6, 4)
    )
    r = flp_report(b.to_bytes())
    assert r["events"] == 6
    assert r["distinct_ids"] == 6
    assert r["blob_bytes"] == 205


# --------------------------------------------------------------------------- #
# varint boundaries
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("size", [0, 1, 126, 127, 128, 129, 16382, 16383, 16384, 16385])
def test_varint_payload_sizes_at_every_width_boundary(flp_report, size):
    """
    127->128 and 16383->16384 are where the varint grows a byte. If the decoder
    mis-handles the continuation bit, the parser reads the payload's first byte
    as an event id and the rest of the file becomes noise.
    """
    b = FlpBuilder().var(209, b"\xcc" * size).byte(1, 42)
    r = flp_report(b.to_bytes())
    assert r["events"] == 2, "stream desynchronised at payload size %d" % size
    assert r["rows"][209]["payload_bytes"] == size
    assert r["rows"][1]["count"] == 1


def test_varint_length_bytes_are_not_counted_as_payload(flp_report):
    """A 2-byte varint prefix must not inflate the 'opaque state' figure."""
    r = flp_report(FlpBuilder().var(209, b"z" * 200).to_bytes())
    assert r["rows"][209]["payload_bytes"] == 200


def test_a_multibyte_varint_is_encoded_the_way_the_parser_reads_it():
    assert encode_varint(300) == b"\xac\x02"
    assert encode_varint(16384) == b"\x80\x80\x01"


def test_opaque_payload_percentage_is_computed_over_the_whole_file(flp_report):
    b = FlpBuilder().var(213, b"\x00" * 10_000).byte(0, 1)
    data = b.to_bytes()
    r = flp_report(data)
    assert r["file_bytes"] == len(data)
    assert r["blob_bytes"] == 10_000
    assert r["blob_pct"] == pytest.approx(100 * 10_000 / len(data), abs=0.05)


# --------------------------------------------------------------------------- #
# text decoding — the suspect heuristic
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize(
    "text",
    ["Kick", "Snare 01", "Bass Drum", "msi-kick (1)", "10.0.0",
     r"%FLStudioData%\Patches\kick.wav"],
)
def test_latin1_text_decodes_correctly(flp, text):
    assert flp.decode_text(latin1_text(text)) == text


@pytest.mark.parametrize(
    "text",
    ["Kick", "Snare 01", "Café au lait", r"%FLStudioData%\Patches\kick.wav", "A"],
)
def test_utf16_text_decodes_correctly(flp, text):
    assert flp.decode_text(utf16_text(text)) == text


def test_empty_text_payload(flp):
    assert flp.decode_text(b"") == ""


def test_the_heuristic_is_what_distinguishes_the_two(flp):
    """
    There is no encoding flag in the format — the same bytes are legal in both
    encodings, so decode_text guesses. Documented here because everything below
    is a consequence of guessing.
    """
    assert flp.decode_text(b"Kick\x00") == "Kick"  # 5 bytes: odd, must be latin-1
    assert flp.decode_text(b"K\x00i\x00c\x00k\x00\x00\x00") == "Kick"


@pytest.mark.parametrize("name", ["Hat", "Kik", "Clp", "Bss", "Ldr"])
@pytest.mark.xfail(
    strict=True,
    reason=(
        "BUG in flp_parse.decode_text: a 3-character latin-1 name plus its NUL "
        "terminator is 4 bytes — an even length, so UTF-16LE decodes it without "
        "error into two code points, one of which is the trailing ASCII char. "
        "The heuristic accepts a result with >= 50% printable-ASCII characters, "
        "and 1 of 2 is exactly 50%, so the mojibake wins: b'Hat\\x00' decodes to "
        "'\u6148t'. Three-letter channel names (Hat, Kik, Clp) are ordinary in FL "
        "projects. Fix: score on the fraction of NON-ASCII code points instead, "
        "or require the UTF-16 reading to contain no characters above U+00FF "
        "unless the payload also has a plausible BOM/odd-byte pattern."
    ),
)
def test_three_letter_latin1_names_survive(flp, name):
    assert flp.decode_text(latin1_text(name)) == name


@pytest.mark.parametrize("text", ["キック", "Кик", "鼓组"])
@pytest.mark.xfail(
    strict=True,
    reason=(
        "BUG in flp_parse.decode_text, the other direction: the heuristic demands "
        "that a successful UTF-16 decode be >= 50% printable ASCII, so genuinely "
        "non-Latin text — Japanese, Cyrillic, Chinese channel names, which FL "
        "supports and writes as UTF-16LE — is rejected and re-read as latin-1 "
        "mojibake. Fix: prefer UTF-16 when the payload length is even AND the "
        "odd-indexed bytes are predominantly 0x00 (ASCII-in-UTF-16), else when "
        "the decode yields no unpaired surrogates and no C0 controls."
    ),
)
def test_non_latin_utf16_names_survive(flp, text):
    assert flp.decode_text(utf16_text(text)) == text


def test_text_events_appear_in_the_report(flp_report):
    b = (
        FlpBuilder()
        .text(199, "10.0.0", encoding="latin-1")
        .text(194, "My Track", encoding="utf-16")
        .text(196, r"%FLStudioData%\kick.wav", encoding="latin-1")
    )
    r = flp_report(b.to_bytes())
    assert ("Version", "10.0.0") in r["texts"]
    assert ("Title", "My Track") in r["texts"]
    assert ("SampleFileName", r"%FLStudioData%\kick.wav") in r["texts"]


def test_fl_path_tokens_are_recognised(flp_report):
    """
    docs/FORMATS.md: FL's %FLStudioData% tokens are "more portable than the
    absolute paths Ableton stores" — the parser calls that out, so it must fire.
    """
    b = FlpBuilder().text(196, r"%FLStudioData%\Patches\kick.wav", encoding="latin-1")
    r = flp_report(b.to_bytes())
    assert "sample references: 1" in r["stdout"]
    assert "%VARIABLE% path tokens" in r["stdout"]


def test_absolute_sample_paths_do_not_trigger_the_token_note(flp_report):
    b = FlpBuilder().text(196, r"C:\Samples\kick.wav", encoding="latin-1")
    r = flp_report(b.to_bytes())
    assert "sample references: 1" in r["stdout"]
    assert "%VARIABLE% path tokens" not in r["stdout"]


def test_blank_text_events_are_dropped_from_the_listing(flp_report):
    b = (
        FlpBuilder()
        .text(194, "", encoding="latin-1")
        .text(195, "    ", encoding="latin-1")
        .text(193, "Real", encoding="latin-1")
    )
    r = flp_report(b.to_bytes())
    assert [t for _, t in r["texts"]] == ["Real"]


def test_non_text_variable_events_are_not_decoded_as_text(flp_report):
    """
    Ids 208+ are binary. Decoding them as text would be both wrong and a way to
    print raw plugin memory into a log.
    """
    b = FlpBuilder().var(209, bytes(range(256))).var(213, b"\xff" * 64)
    r = flp_report(b.to_bytes())
    assert r["texts"] == []
    assert r["blob_bytes"] == 256 + 64


# --------------------------------------------------------------------------- #
# naming and totals
# --------------------------------------------------------------------------- #


def test_known_event_ids_are_named_and_unknown_ones_are_marked(flp_report, flp):
    b = FlpBuilder().byte(0, 1).byte(7, 1)  # 0 = ChanEnabled, 7 = not in the table
    assert 0 in flp.EVENT_NAMES and 7 not in flp.EVENT_NAMES
    r = flp_report(b.to_bytes())
    assert r["rows"][0]["name"] == "ChanEnabled"
    assert r["rows"][7]["name"] == "?"


def test_event_and_distinct_id_totals(flp_report):
    b = FlpBuilder()
    for _ in range(5):
        b.byte(0, 1)
    for _ in range(3):
        b.word(64, 1)
    b.var(209, b"a" * 10)
    r = flp_report(b.to_bytes())
    assert r["events"] == 9
    assert r["distinct_ids"] == 3


def test_only_the_top_twenty_ids_are_listed(flp_report):
    b = FlpBuilder()
    for ev in range(30):
        b.byte(ev, 1)
    r = flp_report(b.to_bytes())
    assert r["events"] == 30
    assert r["distinct_ids"] == 30
    assert len(r["rows"]) == 20, "the report shows most_common(20)"


def test_realistic_project_parses_end_to_end(flp_report):
    built = realistic_project()
    r = flp_report(built.to_bytes())
    assert r["events"] == built.event_count
    assert r["blob_pct"] > 50.0, "opaque plugin state should dominate, as on real files"
