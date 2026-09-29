"""
Tests for experiments/logic_region_map.py.

Every fixture is generated in code. No `ProjectData`, `.logicx` or `.band` byte
is ever committed (CONTRIBUTING.md), so the builders below are the spec for the
layout the parser claims: if a builder and the parser drift apart, these fail.
"""

from __future__ import annotations

import struct

import logic_region_map as lrm
import pytest

# --------------------------------------------------------------------------- #
# builders — the layout under test, written out longhand
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
    """One 48-byte placement group (three 16-byte event units)."""
    event = bytearray(lrm.PLACEMENT_GROUP_LEN)
    event[0:4] = lrm.PLACEMENT_MARKER
    position = lrm.REGION_TIME_ORIGIN + round((bar - 1) * lrm.TICKS_PER_BAR)
    struct.pack_into("<I", event, lrm.PLACEMENT_POSITION_OFFSET, position)
    struct.pack_into("<I", event, lrm.PLACEMENT_EVENT_ID_OFFSET, event_id)
    event[lrm.PLACEMENT_TRACK_OFFSET] = track
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
# gRuA — the region object
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


def test_a_record_too_short_to_hold_a_name_length_is_skipped():
    song = lrm.parse(build_container([build_record(lrm.TAG_REGION, b"\x00" * 8)]))
    assert song.regions == []


def test_a_name_that_is_not_utf8_is_skipped():
    record = bytearray(build_region("Kick", 100, uuid_bytes(1)))
    record[lrm.REGION_NAME_OFFSET : lrm.REGION_NAME_OFFSET + 4] = b"\xff\xfe\xfd\xfc"
    song = lrm.parse(build_container([bytes(record)]))
    assert song.regions == []


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
# qSvE — the 16-byte event grid
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
    assert (song.placements, song.placement_markers_rejected) == ([], 1)


def test_a_marker_positioned_beyond_any_real_song_is_rejected():
    event = bytearray(build_placement(4, 1.0, 1))
    struct.pack_into("<I", event, lrm.PLACEMENT_POSITION_OFFSET, 3_154_082_048)
    song = lrm.parse(build_container([build_event_stream([bytes(event)])]))
    assert (song.placements, song.placement_markers_rejected) == ([], 1)


def test_a_marker_before_the_region_time_origin_is_rejected():
    event = bytearray(build_placement(4, 1.0, 1))
    struct.pack_into("<I", event, lrm.PLACEMENT_POSITION_OFFSET, 12)
    song = lrm.parse(build_container([build_event_stream([bytes(event)])]))
    assert (song.placements, song.placement_markers_rejected) == ([], 1)


def test_a_group_truncated_by_the_end_of_the_payload_is_rejected():
    # A marker in the last 16-byte unit: the group's fields run past the payload.
    stream = build_record(lrm.TAG_EVENTS, lrm.PLACEMENT_MARKER + b"\x00" * 12)
    song = lrm.parse(build_container([stream]))
    assert (song.placements, song.placement_markers_rejected) == ([], 1)


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


def test_a_placement_linking_to_no_region_says_so_rather_than_inventing_a_name():
    song = song_with([], [(1, 1.0, 4)])
    assert lrm.family_label(song, 4) == "<unresolved family 4>"


def test_a_family_with_more_regions_than_placements_reports_the_surplus_unplaced():
    song = song_with(
        [
            ("Deep Down Shaker", 173509, uuid_bytes(1), 0),
            ("Deep Down Shaker.1", 173509, uuid_bytes(2), 0),
        ],
        [],
    )
    assert [r.name for r in lrm.unplaced_regions(song)] == [
        "Deep Down Shaker",
        "Deep Down Shaker.1",
    ]


# --------------------------------------------------------------------------- #
# the diff — keyed on the UUID, which is stable across saves
# --------------------------------------------------------------------------- #


def test_a_region_moving_one_bar_is_the_only_thing_reported():
    old = song_with([("Deep Down Shaker", 173509, uuid_bytes(1), 2)], [(1, 1.0, 2)])
    new = song_with([("Deep Down Shaker", 173509, uuid_bytes(1), 2)], [(1, 5.0, 2)])
    assert lrm.report_diff(old, new) == [
        "  track 1: 'Deep Down Shaker' moved from bar 1 to bar 5"
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


def test_a_changed_placement_count_is_reported_without_pairing_guesses():
    regions = [("Loop", 235200, uuid_bytes(1), 2)]
    old = song_with(regions, [(3, 1.0, 2)])
    new = song_with(regions, [(3, 1.0, 2), (3, 9.0, 2)])
    assert lrm.report_diff(old, new) == ["  track 3: 1 placements -> 2 placements"]


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


def test_the_map_lists_region_objects_that_are_not_on_the_timeline(tmp_path, capsys):
    path = tmp_path / "ProjectData"
    path.write_bytes(
        build_container(
            [
                build_region("Deep Down Shaker", 173509, uuid_bytes(1), family=0),
                build_region("Deep Down Shaker.1", 173509, uuid_bytes(2), family=0),
                build_event_stream([]),
            ]
        )
    )
    assert lrm.main([str(path)]) == 0
    out = capsys.readouterr().out
    assert "REGION OBJECTS NOT ON THE TIMELINE (2)" in out
    assert "  Deep Down Shaker                   173509 frames" in out


def test_only_the_placement_that_moved_is_reported(tmp_path):
    # Two regions on one track; the first stays, the second moves. The unmoved
    # one must not appear -- a false positive here is what blocks the PR.
    regions = [("Loop", 235200, uuid_bytes(1), 2)]
    old = song_with(regions, [(1, 1.0, 2), (1, 5.0, 2)])
    new = song_with(regions, [(1, 1.0, 2), (1, 9.0, 2)])
    assert lrm.report_diff(old, new) == [
        "  track 1: 'Loop' moved from bar 5 to bar 9"
    ]


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
    assert "track 7   bar 12       Deep Down Shaker" in capsys.readouterr().out


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


# --------------------------------------------------------------------------- #
# --scan — library-wide decode rates
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
        totals["regions"],
        totals["placements"],
        totals["placements_resolved_to_a_region"],
        totals["placements_on_the_960_grid"],
        totals["saves_track_count_within_declared"],
    ) == (1, 2, 2, 2, 2, 2, 2, 2)


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
    assert "bundles                      1" in capsys.readouterr().out


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
# real material — loud skip when WIT_FIXTURES is unset
# --------------------------------------------------------------------------- #


@pytest.mark.real_fixtures
def test_real_saves_walk_and_every_event_stream_is_sixteen_byte_aligned(real_material):
    for path in real_material.logic_project_data(min_len=2)[:10]:
        song = lrm.parse(path.read_bytes())
        assert song.event_streams_misaligned == 0


@pytest.mark.real_fixtures
def test_every_real_region_decodes_a_name_and_a_distinct_uuid(real_material):
    for path in real_material.logic_project_data(min_len=2)[:10]:
        song = lrm.parse(path.read_bytes())
        if not song.regions:
            continue
        assert all(r.name for r in song.regions)
        assert len({r.uuid for r in song.regions}) == len(song.regions)


@pytest.mark.real_fixtures
def test_every_real_placement_resolves_to_a_region_family(real_material):
    for path in real_material.logic_project_data(min_len=2)[:10]:
        song = lrm.parse(path.read_bytes())
        if not song.regions:
            continue
        families = song.families()
        assert all(p.family in families for p in song.placements)
