"""
Semantic diff: one change at a time.

Each test makes exactly one edit to a known set and asserts the complete diff.
Asserting the *whole* list, not "some line contains volume", is deliberate: the
value of a semantic diff is that it stays quiet about everything else, and a test
that only checks for presence cannot catch a regression that adds noise.
"""

from __future__ import annotations

import pytest
from factories.als import Clip, LiveSet, Track, simple_set

# --------------------------------------------------------------------------- #
# quiet cases
# --------------------------------------------------------------------------- #


def test_identical_sets_produce_no_changes(diff_of):
    base = simple_set()
    assert diff_of(base, base.copy()) == []


def test_view_state_only_save_reports_no_musical_change(diff_of):
    """
    docs/EXPERIMENTS.md 1: one real save's entire 170-line raw diff was scroll
    position, zoom and selection. That save must produce an empty semantic diff —
    this is the headline claim of the whole approach.
    """
    a = simple_set()
    b = a.copy()
    b.scroll_x = 12_800
    b.zoom = 4.0
    b.selected_track = "101"
    b.next_pointee_id = 50_000
    b.track("Drums").selected = True
    b.track("Drums").clip("10").scroller_time = 44.0
    b.track("Bass").clip("20").warp_markers = 30

    assert a.to_xml_bytes() != b.to_xml_bytes(), "the fixture did not actually change"
    assert diff_of(a, b) == []


# --------------------------------------------------------------------------- #
# tracks
# --------------------------------------------------------------------------- #


def test_track_added(diff_of):
    a = simple_set()
    b = a.copy()
    b.tracks.append(Track(id="103", name="Vox", kind="AudioTrack"))
    assert diff_of(a, b) == ["TRACK+  added 'Vox' (AudioTrack)"]


def test_track_removed(diff_of):
    a = simple_set()
    b = a.copy()
    b.remove_track("Bass")
    assert diff_of(a, b) == ["TRACK-  removed 'Bass'"]


def test_track_renamed_is_not_reported_as_add_plus_remove(diff_of):
    """
    Experiment 2 ("Ableton track IDs are stable") is what makes this possible and
    is called the single most important enabling fact for the project. If Ids
    ever stop being the identity key, this test turns into TRACK+/TRACK-.
    """
    a = simple_set()
    b = a.copy()
    b.track("Bass").name = "Sub Bass"
    assert diff_of(a, b) == ["TRACK~  renamed 'Bass' -> 'Sub Bass'"]


def test_track_reordered_without_edits_is_silent(diff_of):
    """Moving a track in the arrangement changes no musical state."""
    a = simple_set()
    b = a.copy()
    b.tracks = [b.tracks[2], b.tracks[0], b.tracks[1]]
    assert diff_of(a, b) == []


# --------------------------------------------------------------------------- #
# mixer
# --------------------------------------------------------------------------- #


def test_volume_change(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Bass").volume = 0.525
    assert diff_of(a, b) == ["MIX~    [Bass] volume: 0.794 -> 0.525"]


def test_pan_change(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Drums").pan = 0.5
    assert diff_of(a, b) == ["MIX~    [Drums] pan: 0.0 -> 0.5"]


def test_track_muted(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Keys").speaker = False
    assert diff_of(a, b) == ["MIX~    [Keys] output enabled: true -> false"]


def test_track_colour_change(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Keys").color = "7"
    assert diff_of(a, b) == ["MIX~    [Keys] color: 60 -> 7"]


def test_the_new_name_is_used_as_the_label_when_a_track_is_renamed_and_mixed(diff_of):
    a = simple_set()
    b = a.copy()
    bass = b.track("Bass")
    bass.name = "Sub"
    bass.volume = 0.5
    assert diff_of(a, b) == [
        "TRACK~  renamed 'Bass' -> 'Sub'",
        "MIX~    [Sub] volume: 0.794 -> 0.5",
    ]


# --------------------------------------------------------------------------- #
# devices
# --------------------------------------------------------------------------- #


def test_device_added(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Bass").devices = ["Compressor2"]
    assert diff_of(a, b) == ["FX+     [Bass] added: Compressor2"]


def test_device_removed(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Drums").devices = []
    assert diff_of(a, b) == ["FX-     [Drums] removed: Eq8"]


def test_device_replaced_reports_both_sides(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Drums").devices = ["AutoFilter"]
    assert diff_of(a, b) == [
        "FX+     [Drums] added: AutoFilter",
        "FX-     [Drums] removed: Eq8",
    ]


def test_device_chain_reordered(diff_of):
    a = simple_set()
    a.track("Drums").devices = ["Eq8", "Compressor2"]
    b = a.copy()
    b.track("Drums").devices = ["Compressor2", "Eq8"]
    assert diff_of(a, b) == ["FX~     [Drums] device chain reordered"]


def test_a_second_copy_of_an_existing_device_is_reported_as_a_reorder(diff_of):
    """
    Characterisation, not endorsement. `added` is computed with a membership test
    against a list, so adding a *duplicate* device produces neither an add nor a
    remove and falls through to "reordered". A musician who put a second EQ8 on
    the chain is told the chain was reordered. Worth fixing with a multiset diff;
    recorded here so the current behaviour is at least known.
    """
    a = simple_set()
    b = a.copy()
    b.track("Drums").devices = ["Eq8", "Eq8"]
    assert diff_of(a, b) == ["FX~     [Drums] device chain reordered"]


# --------------------------------------------------------------------------- #
# clips
# --------------------------------------------------------------------------- #


def test_clip_moved(diff_of):
    a = simple_set()
    b = a.copy()
    clip = b.track("Drums").clip("11")
    clip.start, clip.end = 20.0, 36.0
    assert diff_of(a, b) == ["CLIP~   [Drums] 'hat loop' 16.0-32.0 -> 20.0-36.0"]


def test_clip_trimmed(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Bass").clip("20").end = 28.0
    assert diff_of(a, b) == ["CLIP~   [Bass] 'sub' 0.0-32.0 -> 0.0-28.0"]


def test_clip_added(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Bass").clips.append(
        Clip(id="21", name="fill", start=32.0, end=36.0, sample="Samples/x.wav")
    )
    assert diff_of(a, b) == ["CLIP+   [Bass] added 'fill' at bar 32.0"]


def test_clip_removed(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Drums").clips = [c for c in b.track("Drums").clips if c.id != "11"]
    assert diff_of(a, b) == ["CLIP-   [Drums] removed 'hat loop' at bar 16.0"]


def test_unnamed_clip_falls_back_to_its_sample_name(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Bass").clips.append(
        Clip(id="21", name="", start=40.0, end=44.0, sample="Samples/Imported/shaker.wav")
    )
    assert diff_of(a, b) == ["CLIP+   [Bass] added 'shaker.wav' at bar 40.0"]


def test_clip_muted_and_unmuted(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Drums").clip("10").disabled = True
    assert diff_of(a, b) == ["CLIP~   [Drums] 'kick loop' muted"]
    assert diff_of(b, a) == ["CLIP~   [Drums] 'kick loop' unmuted"]


def test_moving_a_clip_to_another_track_is_a_remove_plus_an_add(diff_of):
    a = simple_set()
    b = a.copy()
    moved = b.track("Drums").clip("11")
    b.track("Drums").clips.remove(moved)
    b.track("Bass").clips.append(moved)
    # Reported per track, in track-Id order: Drums (100) loses it, Bass (101)
    # gains it. Nothing links the two halves — a clip move across tracks is not
    # recognised as one event.
    assert diff_of(a, b) == [
        "CLIP-   [Drums] removed 'hat loop' at bar 16.0",
        "CLIP+   [Bass] added 'hat loop' at bar 16.0",
    ]


# --------------------------------------------------------------------------- #
# tempo, automation, notes
# --------------------------------------------------------------------------- #


def test_tempo_change(diff_of):
    a = simple_set()
    b = a.copy()
    b.tempo = 128.0
    assert diff_of(a, b) == ["TEMPO   120.0 -> 128.0 BPM"]


def test_automation_lane_added(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Drums").automation_lanes = 2
    assert diff_of(a, b) == ["AUTO~   [Drums] automation lanes 0 -> 2"]


def test_midi_notes_added(diff_of):
    a = simple_set()
    b = a.copy()
    b.track("Keys").clip("30").notes = 20
    assert diff_of(a, b) == ["MIDI~   [Keys] note count 12 -> 20"]


def test_editing_midi_notes_without_changing_the_count_is_invisible(diff_of):
    """
    Known and documented limit ("MIDI note-level diffs are not produced"). Pinned
    so that if someone implements note diffing they see this test and update it,
    and so nobody mistakes the silence for a passing feature.
    """
    a = simple_set()
    b = a.copy()
    b.track("Keys").clip("30").notes = 12  # same count, would be different pitches
    assert diff_of(a, b) == []


# --------------------------------------------------------------------------- #
# combinations
# --------------------------------------------------------------------------- #


def test_independent_edits_on_different_tracks_do_not_interfere(diff_of):
    """
    Experiment 3: "edits are local". Two disjoint edits must produce exactly two
    lines, in track-Id order.
    """
    a = simple_set()
    b = a.copy()
    b.track("Drums").volume = 0.6
    b.track("Keys").automation_lanes = 3
    assert diff_of(a, b) == [
        "MIX~    [Drums] volume: 1.0 -> 0.6",
        "AUTO~   [Keys] automation lanes 1 -> 3",
    ]


def test_a_realistic_multi_change_save(diff_of):
    """The shape of a real save, from docs/EXPERIMENTS.md 4."""
    a = simple_set()
    b = a.copy()
    b.track("Bass").volume = 0.525
    clip = b.track("Drums").clip("11")
    clip.end = 30.0
    b.track("Drums").clips = [c for c in b.track("Drums").clips if c.id != "10"]

    assert diff_of(a, b) == [
        "CLIP-   [Drums] removed 'kick loop' at bar 0.0",
        "CLIP~   [Drums] 'hat loop' 16.0-32.0 -> 16.0-30.0",
        "MIX~    [Bass] volume: 0.794 -> 0.525",
    ]


def test_changes_are_grouped_by_track_in_id_order(diff_of):
    a = simple_set()
    b = a.copy()
    for track in b.tracks:
        track.volume = round(track.volume - 0.1, 3)
    labels = [line.split("[")[1].split("]")[0] for line in diff_of(a, b)]
    assert labels == ["Drums", "Bass", "Keys"]


@pytest.mark.parametrize("kind", ["AudioTrack", "MidiTrack", "GroupTrack", "ReturnTrack"])
def test_every_track_kind_diffs_the_same_way(diff_of, kind):
    a = LiveSet(tracks=[Track(id="5", name="T", kind=kind, volume=1.0)])
    b = a.copy()
    b.track("5").volume = 0.5
    assert diff_of(a, b) == ["MIX~    [T] volume: 1.0 -> 0.5"]
