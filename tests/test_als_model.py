"""
The semantic model: what ``build_model`` extracts from a Live set.

AGENTS.md: "Prefer whitelist extraction over blacklist normalisation." The
whitelist is the contract — if a field silently stops being extracted, every diff
downstream quietly reports less than it should and no other test would fail. So
each field is asserted individually, including the ones that must be ignored.
"""

from __future__ import annotations

import gzip

import pytest
from factories.als import Clip, LiveSet, Track, simple_set

# --------------------------------------------------------------------------- #
# what is extracted
# --------------------------------------------------------------------------- #


def test_creator_string_is_kept(model_of):
    ls = simple_set()
    ls.creator = "Ableton Live 11.3.21"
    assert model_of(ls)["creator"] == "Ableton Live 11.3.21"


def test_only_recognised_track_tags_become_tracks(model_of, als_diff, write_als, tmp_path):
    """
    A Live set's <Tracks> element also carries things that are not tracks. The
    model must ignore them rather than inventing tracks with Id=None.
    """
    ls = LiveSet(tracks=[Track(id="1", name="Real")])
    xml = ls.to_xml_bytes().replace(
        b"</Tracks>", b'<PreHearTrack Id="99"><Name><EffectiveName Value="cue"/></Name>'
        b"</PreHearTrack></Tracks>"
    )
    path = tmp_path / "extra.als"
    with gzip.GzipFile(str(path), "wb", mtime=0) as fh:
        fh.write(xml)

    model = als_diff.build_model(str(path))
    assert set(model["tracks"]) == {"1"}


@pytest.mark.parametrize(
    "value,expected",
    [
        (1.0, 1.0),
        (0.0, 0.0),
        (0.7943282127, 0.794),
        (0.5254999995, 0.525),
        (-0.25, -0.25),
        (0.0005, 0.001),
    ],
)
def test_mixer_values_are_rounded_to_three_places(model_of, value, expected):
    """
    Rounding is the noise gate: Live rewrites floats with tiny differences on
    save, and without it every save would report a volume change.
    """
    ls = LiveSet(tracks=[Track(id="1", name="T", volume=value)])
    assert model_of(ls)["tracks"]["1"]["volume"] == expected


def test_a_sub_millesimal_volume_wobble_is_not_a_change(diff_of):
    a = LiveSet(tracks=[Track(id="1", name="T", volume=0.7943282127)])
    b = a.copy()
    b.track("1").volume = 0.7943282999
    assert diff_of(a, b) == []


def test_sample_reference_is_reduced_to_a_basename(model_of):
    """
    The model keys samples by basename on purpose: the full RelativePath differs
    between machines, and real sets embed other people's home directories.
    """
    ls = LiveSet(
        tracks=[
            Track(
                id="1",
                name="T",
                clips=[Clip(id="9", sample="Samples/Processed/Consolidate/take 3.wav")],
            )
        ]
    )
    assert model_of(ls)["tracks"]["1"]["clips"]["9"]["sample"] == "take 3.wav"


def test_missing_sample_reference_becomes_empty_string(model_of):
    ls = LiveSet(tracks=[Track(id="1", name="T", clips=[Clip(id="9", sample="")])])
    assert model_of(ls)["tracks"]["1"]["clips"]["9"]["sample"] == ""


def test_track_without_a_name_falls_back_to_a_placeholder(model_of, als_diff, tmp_path):
    ls = LiveSet(tracks=[Track(id="1", name="T")])
    xml = ls.to_xml_bytes().replace(b'<EffectiveName Value="T" />', b"")
    path = tmp_path / "noname.als"
    with gzip.GzipFile(str(path), "wb", mtime=0) as fh:
        fh.write(xml)
    assert als_diff.build_model(str(path))["tracks"]["1"]["name"] == "?"


def test_device_chain_is_recorded_in_order(model_of):
    ls = LiveSet(
        tracks=[Track(id="1", name="T", devices=["Eq8", "Compressor2", "AutoFilter"])]
    )
    assert model_of(ls)["tracks"]["1"]["devices"] == ["Eq8", "Compressor2", "AutoFilter"]


def test_automation_lanes_and_notes_are_counted(model_of):
    ls = LiveSet(
        tracks=[
            Track(
                id="1",
                name="T",
                kind="MidiTrack",
                automation_lanes=3,
                clips=[Clip(id="9", notes=7), Clip(id="10", notes=5)],
            )
        ]
    )
    track = model_of(ls)["tracks"]["1"]
    assert track["automation_lanes"] == 3
    assert track["notes"] == 12


def test_sample_index_maps_basename_to_the_tracks_using_it(model_of):
    ls = LiveSet(
        tracks=[
            Track(id="1", name="Drums", clips=[Clip(id="9", sample="a/kick.wav")]),
            Track(id="2", name="Perc", clips=[Clip(id="9", sample="b/kick.wav")]),
        ]
    )
    assert model_of(ls)["samples"]["kick.wav"] == {"Drums", "Perc"}


# --------------------------------------------------------------------------- #
# what must NOT be extracted
# --------------------------------------------------------------------------- #


def test_view_state_never_reaches_the_model(model_of):
    """
    Experiment 1 measured a 170-line raw diff that was "entirely scroll position,
    zoom level and selection state". That noise must not survive into the model.
    """
    a = simple_set()
    b = a.copy()
    b.scroll_x = 88_000
    b.zoom = 7.25
    b.selected_track = "102"
    b.tracks[0].selected = True
    b.tracks[0].clips[0].scroller_time = 31.5
    b.next_pointee_id = 99_999

    assert a.to_xml_bytes() != b.to_xml_bytes()
    assert model_of(a) == model_of(b)


def test_warp_markers_do_not_reach_the_model(model_of):
    """
    5,112 warp markers in the reference project. They are churn for this model;
    if they ever start mattering the model has changed meaning.
    """
    a = LiveSet(tracks=[Track(id="1", name="T", clips=[Clip(id="9", warp_markers=2)])])
    b = a.copy()
    b.track("1").clip("9").warp_markers = 40
    assert model_of(a) == model_of(b)


# --------------------------------------------------------------------------- #
# tempo
# --------------------------------------------------------------------------- #


def test_tempo_is_read_from_the_master_track(model_of):
    ls = simple_set()
    ls.tempo = 128.0
    ls.tempo_host_tag = "MasterTrack"
    assert model_of(ls)["tempo"] == 128.0


@pytest.mark.xfail(
    strict=True,
    reason=(
        "BUG in als_semantic_diff.build_model: it looks for LiveSet/MasterTrack, "
        "but Live 12.3 writes LiveSet/MainTrack. Verified against a real Live "
        "12.3.5 set on the development machine: build_model returns tempo=None "
        "for every one of the 30 autosaves, so a tempo change is invisible on "
        "current Ableton — while the module docstring claims 'tested against "
        "Live 12.x'. Fix: accept either tag (or search LiveSet/.//Tempo/Manual)."
    ),
)
def test_tempo_is_read_from_a_live_12_3_main_track(model_of):
    ls = simple_set()
    ls.tempo = 128.0
    ls.tempo_host_tag = "MainTrack"
    assert model_of(ls)["tempo"] == 128.0


def test_a_set_with_no_tempo_host_reports_none_rather_than_crashing(model_of):
    ls = LiveSet(tracks=[Track(id="1", name="T")], tempo_host_tag="MainTrack")
    assert model_of(ls)["tempo"] is None


# --------------------------------------------------------------------------- #
# structural robustness of the extractor
# --------------------------------------------------------------------------- #


def test_a_track_with_no_clips_yields_an_empty_clip_map(model_of):
    ls = LiveSet(tracks=[Track(id="1", name="Empty")])
    assert model_of(ls)["tracks"]["1"]["clips"] == {}


def test_an_empty_set_has_no_tracks(model_of):
    assert model_of(LiveSet(tracks=[]))["tracks"] == {}


def test_a_track_without_a_device_chain_does_not_crash(als_diff, tmp_path):
    """
    Return tracks and some group tracks in older schemas can lack the elements the
    model reaches for. `_val` is written to tolerate that; prove it.
    """
    ls = LiveSet(tracks=[Track(id="1", name="Odd")])
    xml = ls.to_xml_bytes()
    start = xml.index(b"<DeviceChain>")
    end = xml.index(b"</DeviceChain></") + len(b"</DeviceChain>")
    stripped = xml[:start] + xml[end:]
    path = tmp_path / "nochain.als"
    with gzip.GzipFile(str(path), "wb", mtime=0) as fh:
        fh.write(stripped)

    track = als_diff.build_model(str(path))["tracks"]["1"]
    assert track["volume"] is None
    assert track["clips"] == {}
    assert track["devices"] == []
