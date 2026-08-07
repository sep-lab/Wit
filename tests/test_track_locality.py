"""
Unit tests for ``experiments/track_locality.py`` (issue #11).

These pin the two properties the script's docstring claims for its frozen
blacklist, using synthetic sets (never a committed .als, per AGENTS.md):

1. A track that did not change hashes equal, under both methods.
2. A real content change (mixer field, clip move) is caught by both methods.
3. Pure view state, and warp-marker noise, changes neither hash.
4. The specific bug the blacklist's ``TRACK_BLACKLIST_ATTRS`` exists to avoid:
   editing one track must not falsely flag an untouched track just because
   editing upstream shifted a positional id (``AutomationTarget``/
   ``ModulationTarget``) elsewhere in the file.

It does not re-verify the real-material numbers in docs/EXPERIMENTS.md Sec 3
-- those come from running the script itself against real autosaves, per its
own module docstring, and are not reproducible from a synthetic fixture.
"""

from __future__ import annotations

import track_locality
from factories.als import simple_set


def _compare(write_als, a, b):
    pa = write_als(a, "a.als")
    pb = write_als(b, "b.als")
    ta = track_locality.load_tracks(pa)
    tb = track_locality.load_tracks(pb)
    changed_bl, total = track_locality.compare(ta, tb, track_locality.blacklist_hash)
    changed_wl, _ = track_locality.compare(ta, tb, track_locality.whitelist_hash)
    return changed_bl, changed_wl, total


def test_identical_sets_change_nothing(write_als):
    a = simple_set()
    b = a.copy()
    changed_bl, changed_wl, total = _compare(write_als, a, b)
    assert changed_bl == set()
    assert changed_wl == set()
    assert len(total) == 3


def test_mixer_change_is_caught_by_both_methods(write_als):
    a = simple_set()
    b = a.copy()
    b.track("Bass").volume = 0.331
    changed_bl, changed_wl, _ = _compare(write_als, a, b)
    assert changed_bl == {"101"}
    assert changed_wl == {"101"}


def test_clip_move_is_caught_by_both_methods(write_als):
    a = simple_set()
    b = a.copy()
    clip = b.track("Drums").clip("11")
    clip.start, clip.end = 20.0, 36.0
    changed_bl, changed_wl, _ = _compare(write_als, a, b)
    assert changed_bl == {"100"}
    assert changed_wl == {"100"}


def test_view_state_only_change_is_invisible_to_both_methods(write_als):
    a = simple_set()
    b = a.copy()
    # Selection/scroll/zoom -- exactly the fields TRACK_BLACKLIST_ELEMENTS
    # names, and fields the whitelist never reads in the first place.
    b.track("Drums").selected = True
    b.scroll_x = 999
    b.zoom = 8.5
    b.selected_track = "Drums"
    changed_bl, changed_wl, _ = _compare(write_als, a, b)
    assert changed_bl == set()
    assert changed_wl == set()


def test_warp_marker_only_change_is_invisible_to_both_methods(write_als):
    a = simple_set()
    b = a.copy()
    # Simulates Live re-analysing the same audio and re-placing warp markers
    # with no user edit -- see the WarpMarker entry in TRACK_BLACKLIST_ELEMENTS.
    b.track("Drums").clip("10").warp_markers = 99
    changed_bl, changed_wl, _ = _compare(write_als, a, b)
    assert changed_bl == set()
    assert changed_wl == set()


def test_upstream_edit_does_not_falsely_flag_an_untouched_track(write_als):
    """
    The bug TRACK_BLACKLIST_ATTRS exists to avoid.

    Adding a device to 'Drums' shifts the positional AutomationTarget /
    ModulationTarget id counter threaded through every later track's mixer
    -- exactly the "FileRef id shifts +1 when anything upstream is inserted"
    shape measured in docs/EXPERIMENTS.md Sec 1. Bass and Keys are otherwise
    byte-for-byte identical and must not be reported as changed by either
    method.
    """
    a = simple_set()
    b = a.copy()
    b.track("Drums").devices.append("Reverb")
    changed_bl, changed_wl, total = _compare(write_als, a, b)
    assert changed_bl == {"100"}
    assert changed_wl == {"100"}
    assert total == {"100", "101", "102"}


def test_added_and_removed_tracks_count_as_changed(write_als):
    a = simple_set()
    b = a.copy()
    b.remove_track("Keys")
    changed_bl, changed_wl, total = _compare(write_als, a, b)
    assert changed_bl == {"102"}
    assert changed_wl == {"102"}
    assert total == {"100", "101", "102"}
