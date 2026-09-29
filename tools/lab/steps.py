"""
steps — the Rosetta lab's edit script (PLAN-V2 Phase B), as data.

WHAT THIS DOES
    Holds the 26-row edit script — one known edit per save — with, for each step:
    its plan row number `n` (1-26) and a sub-letter where the plan puts two edits
    on one row (e.g. 17a pan L50 / 17b pan R50), a stable `id`, the edit and its
    purpose, the EXPECTED change in machine-checkable form, which DAWs it applies
    to, and a per-DAW instruction the computer-use driver follows (menu paths and
    shortcuts are hints; the driver records any deviation with
    `capture.py watch --note`).

    Rows the plan combines are split so every save carries exactly one edit:
    8a/8b (Persian track name, then a Persian audio FILE name — the plan's Unicode
    risk covers both), 17a/17b, 18a/18b, 23a/23b, 25a/25b. Keys look like "08a".

    Tracks are named unambiguously. 8a renames the click-drums track to آواز;
    8b imports a file with a DIFFERENT Persian name, صدا.wav, because FL Studio
    (and possibly others) names a new track or channel after the imported file —
    a file called آواز.wav would create a second "آواز" and make every later
    "the آواز track" instruction ambiguous (step 24 could then delete the track
    holding all the edits). Track targets therefore say what the track HOLDS
    ({"track": "آواز", "holds": "click-drums.wav"}), and validate() proves that no
    two tracks can ever share a name (adds_track / renames_track / deletes_track).

    Pan (17a/17b) is the same MUSICAL position in every DAW — half-way between
    centre and hard left/right — written in each DAW's native units: Logic -32/+32
    on its -64..+63 knob, Live 25L/25R on its 50L..50R scale (Live's 50L is HARD
    left), FL 50% left/right on its ±100% knob. expected_by_daw records the native
    value and scale next to the shared {"position", "fraction"}.

    Applicability: Logic runs everything; Ableton everything but the Logic-only
    Alternative (25b); FL Studio 20 everything it supports (no project key, no
    Alternatives); GarageBand 12 steps (the plan says "about 10": it shares
    Logic's parser but carries no region records, so arrangement, plugin and
    routing steps are skipped). Every non-applicable DAW has a stated reason.

    EXPECTED_KEYS is the vocabulary of expectations. Values are exact where the
    edit sets an exact value (tempo 124, volume -6 dB, pan half left) and
    descriptive where the DAW picks (a new track's default name is not asserted).

USAGE
    python3 tools/lab/steps.py                  # the whole script, all DAWs
    python3 tools/lab/steps.py --daw logic      # what the driver does in Logic
    python3 tools/lab/steps.py --json           # machine-readable
    python3 tools/lab/steps.py --check          # validate the data; exit 1 on a problem

    capture.py copies the applicable steps into each run's manifest.json.

WHAT THIS DOES NOT HANDLE
    - The instructions are hints written without launching any DAW. Menu names
      drift between versions (Logic 12.3.1, Live 12.4.2, FL 20.8, GarageBand
      10.4.14 are the targets); the driver adapts and notes what it really did.
    - Expected values describe the intended edit, not what Wit can see. Whether
      a DAW's file actually records it is exactly what the corpus will measure.
    - Bar arithmetic after steps 3-4 (124 BPM, 3/4) is the DAW's; the 4-bar
      click file was rendered at 120 BPM 4/4, so it no longer lines up with bars.
"""

from __future__ import annotations

import argparse
import json
import sys
import unicodedata
from typing import Dict, List, Optional

SCRIPT_VERSION = 1

EXPECTED_KEYS = {
    "baseline", "none",
    "tempo_bpm", "time_signature", "key",
    "track_added", "channel_added", "region_added", "audio_file_added",
    "track_renamed", "channel_renamed",
    "region_moved_bars", "region_trimmed", "region_duplicated", "region_deleted",
    "midi_notes", "pattern_added",
    "volume_db", "pan", "track_muted", "track_soloed",
    "plugin_added", "plugin_param_changed", "automation_added",
    "marker_added", "track_color_changed", "send_added",
    "track_deleted", "channel_deleted",
    "project_copied", "alternative_added", "bounce_exported",
}

SAVE_HINT = {
    "logic": "Save: File ▸ Save (⌘S).",
    "garageband": "Save: File ▸ Save (⌘S).",
    "ableton": "Save: File ▸ Save Live Set (⌘S).",
    "fl": "Save: File ▸ Save (⌘S).",
}

PERSIAN = unicodedata.normalize("NFC", "آواز")  # the track name set in 8a ("song, voice")
# The synthetic Persian-named FILE imported in 8b ("sound"). It must differ from
# PERSIAN: FL names a new channel after the imported file (see the module docstring).
PERSIAN_FILE = unicodedata.normalize("NFC", "صدا.wav")
MAIN_AUDIO = "click-drums.wav"
MIDI_13 = "the MIDI region from step 13"
TRACK = "%s (click-drums)" % PERSIAN  # how instructions name the track all later edits go to
TRACK_TARGET = {"track": PERSIAN, "holds": MAIN_AUDIO}

# Each step: n, sub, id, edit, purpose, expected, daws{daw: instruction},
# optional: not_applicable{daw: reason}, expected_by_daw{daw: {...}}, target, allow_identical,
# and the track bookkeeping validate() checks: adds_track (what the new track holds),
# renames_track {"holds", "to"}, deletes_track (what the deleted track holds).
STEPS: List[Dict] = [
    {
        "n": 1, "sub": "", "id": "baseline",
        "edit": "Create a new empty project and Save As into the lab run folder",
        "purpose": "Baseline",
        "expected": {"baseline": True},
        "daws": {
            "logic": "File ▸ New (⇧⌘N) ▸ Empty Project. If Logic insists on a first track, create one "
                     "Audio track and say so with `watch --note`. File ▸ Save As… into "
                     "~/WitLab/logic/<run>/ as `Lab` (organise as a package, copy audio into the "
                     "project). NEVER accept the default ~/Music/Logic location.",
            "ableton": "File ▸ New Live Set (⌘N). File ▸ Save Live Set As… into ~/WitLab/ableton/<run>/ "
                       "as `Lab` (Live creates `Lab Project/Lab.als`).",
            "fl": "File ▸ New (FL's default factory template). File ▸ Save as… into ~/WitLab/fl/<run>/ as "
                  "`Lab.flp`. Never save into FL's own Projects folder.",
            "garageband": "File ▸ New ▸ Empty Project. GarageBand asks for a first track: choose Audio and "
                          "say so with `watch --note`. File ▸ Save As… into ~/WitLab/garageband/<run>/ as "
                          "`Lab.band`. NEVER accept the default ~/Music/GarageBand location.",
        },
    },
    {
        "n": 2, "sub": "", "id": "save-no-change",
        "edit": "Save again without changing anything",
        "purpose": "Pure churn: what a save with no edit changes on disk",
        "expected": {"none": True},
        "allow_identical": True,
        "daws": dict.fromkeys(
            ("logic", "ableton", "fl", "garageband"),
            "Change nothing. Only save. If the DAW disables Save because nothing changed, run "
            "`capture.py next --skip 'save disabled when unmodified'` instead of editing anything.",
        ),
    },
    {
        "n": 3, "sub": "", "id": "tempo-124",
        "edit": "Change the tempo from 120 to 124 BPM",
        "purpose": "Tempo",
        "expected": {"tempo_bpm": 124},
        "daws": {
            "logic": "Double-click the tempo in the control-bar LCD, type 124, Return.",
            "ableton": "Click the tempo field in the control bar, type 124, Return.",
            "fl": "Right-click the tempo in the transport panel ▸ Type in value, 124, Return (or drag it to 124.000).",
            "garageband": "Double-click the tempo in the LCD, type 124, Return.",
        },
    },
    {
        "n": 4, "sub": "", "id": "time-signature-3-4",
        "edit": "Change the time signature from 4/4 to 3/4",
        "purpose": "Meter",
        "expected": {"time_signature": "3/4"},
        "daws": {
            "logic": "Click the time signature in the LCD and set it to 3/4.",
            "ableton": "Click the time-signature numerator in the control bar, type 3, Return.",
            "fl": "Options ▸ Project general settings ▸ Time settings: set the time signature to 3/4.",
            "garageband": "Click the time signature in the LCD and set it to 3/4.",
        },
    },
    {
        "n": 5, "sub": "", "id": "key-d-minor",
        "edit": "Set the project key / scale to D minor",
        "purpose": "Key",
        "expected": {"key": {"tonic": "D", "mode": "minor"}},
        "daws": {
            "logic": "Click the key in the LCD and choose D minor.",
            "ableton": "Turn on Scale Mode in the control bar; set Root D and Scale Minor.",
            "garageband": "Click the key in the LCD and choose D minor.",
        },
        "not_applicable": {"fl": "FL Studio 20 has no project key or scale setting."},
    },
    {
        "n": 6, "sub": "", "id": "add-audio-track",
        "edit": "Add an audio track and import click-drums.wav at bar 1",
        "adds_track": MAIN_AUDIO,
        "purpose": "Track added; audio region and audio-file record",
        "expected": {"track_added": {"kind": "audio"},
                     "region_added": {"file": "click-drums.wav", "start_bar": 1}},
        "expected_by_daw": {
            "fl": {"channel_added": {"kind": "audio_clip"},
                   "region_added": {"file": "click-drums.wav", "start_bar": 1}},
        },
        "daws": {
            "logic": "Track ▸ New Audio Track (⌥⌘A). File ▸ Import ▸ Audio File… → "
                     "~/WitLab/audio/click-drums.wav, placed at bar 1. If Logic offers to import the "
                     "file's tempo, decline.",
            "ableton": "Create ▸ Insert Audio Track (⌘T). Drag ~/WitLab/audio/click-drums.wav onto that track "
                       "at bar 1 in Arrangement View (drag from Finder needs Finder approved; adding a "
                       "Places folder changes Live's browser settings, so ask Sepehr first).",
            "fl": "Drag ~/WitLab/audio/click-drums.wav into the Playlist at bar 1 (FL makes an Audio Clip channel).",
            "garageband": "Track ▸ New Track ▸ Audio. Drag ~/WitLab/audio/click-drums.wav to bar 1 of that track.",
        },
    },
    {
        "n": 7, "sub": "", "id": "rename-track-drums",
        "edit": "Rename that track (it holds click-drums) to Drums",
        "renames_track": {"holds": MAIN_AUDIO, "to": "Drums"},
        "purpose": "Rename; track-name mapping for Logic",
        "expected": {"track_renamed": {"to": "Drums"}},
        "expected_by_daw": {"fl": {"channel_renamed": {"to": "Drums"}}},
        "daws": {
            "logic": "Double-click the track name in the track header, type Drums, Return.",
            "ableton": "Select the track, Edit ▸ Rename (⌘R), type Drums, Return.",
            "fl": "Channel rack: right-click the click-drums channel ▸ Rename, type Drums, Return.",
            "garageband": "Double-click the track name, type Drums, Return.",
        },
    },
    {
        "n": 8, "sub": "a", "id": "rename-track-persian",
        "edit": "Rename the Drums track (it holds click-drums) to %s (Persian)" % PERSIAN,
        "renames_track": {"holds": MAIN_AUDIO, "to": PERSIAN},
        "purpose": "Unicode track name",
        "expected": {"track_renamed": {"from": "Drums", "to": PERSIAN}},
        "expected_by_daw": {"fl": {"channel_renamed": {"from": "Drums", "to": PERSIAN}}},
        "daws": {
            daw: "Rename the Drums %s exactly as in step 7, typing %s. If typing Persian is awkward, put it "
                 "on the clipboard and paste." % ("channel" if daw == "fl" else "track", PERSIAN)
            for daw in ("logic", "ableton", "fl", "garageband")
        },
    },
    {
        "n": 8, "sub": "b", "id": "import-persian-file",
        "edit": "Add a second audio track and import %s at bar 1 (leave the track's default name; FL "
                "names it after the file, %s — never %s)" % (PERSIAN_FILE, PERSIAN_FILE[:-4], PERSIAN),
        "adds_track": PERSIAN_FILE,
        "purpose": "Unicode audio FILE name (Logic stores audio file names as UTF-16LE)",
        "expected": {"track_added": {"kind": "audio"}, "audio_file_added": PERSIAN_FILE},
        "expected_by_daw": {"fl": {"channel_added": {"kind": "audio_clip"}, "audio_file_added": PERSIAN_FILE}},
        "daws": {
            "logic": "Track ▸ New Audio Track (⌥⌘A); File ▸ Import ▸ Audio File… → ~/WitLab/audio/%s at bar 1."
                     % PERSIAN_FILE,
            "ableton": "Create ▸ Insert Audio Track (⌘T); drag ~/WitLab/audio/%s onto it at bar 1." % PERSIAN_FILE,
            "fl": "Drag ~/WitLab/audio/%s into an empty Playlist track at bar 1 (FL names the new channel %s)."
                  % (PERSIAN_FILE, PERSIAN_FILE[:-4]),
            "garageband": "Track ▸ New Track ▸ Audio; drag ~/WitLab/audio/%s to bar 1 of it." % PERSIAN_FILE,
        },
    },
    {
        "n": 9, "sub": "", "id": "move-region-2-bars",
        "edit": "Move the click-drums region 2 bars later",
        "purpose": "Arrangement: move",
        "expected": {"region_moved_bars": 2},
        "target": {"region": MAIN_AUDIO},
        "daws": {
            "logic": "Select the click-drums region; in the region inspector raise Position by 2 bars (or drag it 2 bars right).",
            "ableton": "Arrangement View: move the click-drums clip 2 bars later.",
            "fl": "Playlist: drag the click-drums clip 2 bars to the right.",
        },
        "not_applicable": {"garageband": "GarageBand saves carry no region records (plan, Phase B); arrangement steps skipped."},
    },
    {
        "n": 10, "sub": "", "id": "trim-region",
        "edit": "Trim the end of the click-drums region by 1 bar",
        "purpose": "Arrangement: trim",
        "expected": {"region_trimmed": {"end_bars": -1}},
        "target": {"region": MAIN_AUDIO},
        "daws": {
            "logic": "Drag the region's lower-right edge 1 bar to the left (or shorten Length by 1 bar in the inspector).",
            "ableton": "Drag the clip's right edge 1 bar to the left.",
            "fl": "Drag the clip's right edge 1 bar to the left.",
        },
        "not_applicable": {"garageband": "GarageBand saves carry no region records; arrangement steps skipped."},
    },
    {
        "n": 11, "sub": "", "id": "duplicate-region",
        "edit": "Duplicate the click-drums region once (the copy lands right after it)",
        "purpose": "Arrangement: duplicate",
        "expected": {"region_duplicated": 1},
        "target": {"region": MAIN_AUDIO},
        "daws": {
            "logic": "Select the region, Edit ▸ Repeat ▸ Once (⌘R).",
            "ableton": "Select the clip, Edit ▸ Duplicate (⌘D).",
            "fl": "Select the clip in the Playlist and duplicate it (⌘B).",
        },
        "not_applicable": {"garageband": "GarageBand saves carry no region records; arrangement steps skipped."},
    },
    {
        "n": 12, "sub": "", "id": "delete-region",
        "edit": "Delete the duplicate made in step 11 (keep the original)",
        "purpose": "Arrangement: delete",
        "expected": {"region_deleted": 1},
        "daws": {
            "logic": "Select only the duplicate region and press Delete.",
            "ableton": "Select only the duplicate clip and press Delete.",
            "fl": "Right-click only the duplicate clip in the Playlist to delete it.",
        },
        "not_applicable": {"garageband": "GarageBand saves carry no region records; arrangement steps skipped."},
    },
    {
        "n": 13, "sub": "", "id": "midi-track",
        "adds_track": MIDI_13,
        "edit": "Add an instrument track with the stock default instrument and a 1-bar MIDI region at bar 1 "
                "holding three quarter notes: middle C, E, G (MIDI 60, 64, 67)",
        "purpose": "MIDI",
        "expected": {"track_added": {"kind": "instrument"}, "midi_notes": [60, 64, 67]},
        "expected_by_daw": {
            "fl": {"channel_added": {"kind": "instrument"}, "pattern_added": 1, "midi_notes": [60, 64, 67]},
        },
        "daws": {
            "logic": "Track ▸ New Software Instrument Track (⌥⌘S), default patch. Create a 1-bar MIDI region at "
                     "bar 1 and draw C3 E3 G3 (Logic's C3 = MIDI 60) as quarter notes in the Piano Roll.",
            "ableton": "Create ▸ Insert MIDI Track (⇧⌘T), load the stock Drift instrument. Insert a 1-bar MIDI "
                       "clip at bar 1 (⇧⌘M) and draw C3 E3 G3 (Live's C3 = MIDI 60) as quarter notes.",
            "fl": "Channel rack ▸ + ▸ FL Keys (stock). In a new pattern draw C5 E5 G5 (FL's C5 = MIDI 60) as "
                  "quarter notes; place the pattern at bar 1 in the Playlist.",
            "garageband": "Track ▸ New Track ▸ Software Instrument (default patch). Create a 1-bar MIDI region at "
                          "bar 1 and draw C3 E3 G3 (C3 = MIDI 60) as quarter notes in the Piano Roll.",
        },
    },
    {
        "n": 14, "sub": "", "id": "volume-minus-6",
        "edit": "Set the %s track's volume to -6 dB" % TRACK,
        "purpose": "Volume sweep: decode Logic's fader encoding",
        "expected": {"volume_db": -6},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "On the %s channel strip, double-click the fader value and type -6, Return." % TRACK,
            "ableton": "Click the %s track's Volume value, type -6, Return." % TRACK,
            "fl": "Set the %s channel's volume to -6.0 dB (hover to read dB in the hint bar; right-click ▸ "
                  "Type in value if offered)." % TRACK,
            "garageband": "Drag the %s track's volume slider to -6.0 dB (read the help tag)." % TRACK,
        },
    },
    {
        "n": 15, "sub": "", "id": "volume-minus-12",
        "edit": "Set the %s track's volume to -12 dB" % TRACK,
        "purpose": "Volume sweep",
        "expected": {"volume_db": -12},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Fader value -12, as in step 14.",
            "ableton": "Volume -12, as in step 14.",
            "fl": "Channel volume -12.0 dB, as in step 14.",
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps; step 14 already covers the fader."},
    },
    {
        "n": 16, "sub": "", "id": "volume-plus-3",
        "edit": "Set the %s track's volume to +3 dB" % TRACK,
        "purpose": "Volume sweep (above unity)",
        "expected": {"volume_db": 3},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Fader value +3, as in step 14.",
            "ableton": "Volume +3, as in step 14.",
            "fl": "Channel volume +3.0 dB if the knob allows it; otherwise its maximum, noted with --note.",
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps; step 14 already covers the fader."},
    },
    {
        "n": 17, "sub": "a", "id": "pan-half-left",
        "edit": "Pan the %s track half-way left (the plan's \"L50\": half of full left, in each DAW's units)" % TRACK,
        "purpose": "Pan encoding; the same musical position in native units per DAW",
        "expected": {"pan": {"position": "half left", "fraction": -0.5}},
        "expected_by_daw": {
            "logic": {"pan": {"position": "half left", "fraction": -0.5, "native": -32, "scale": "-64..+63"}},
            "ableton": {"pan": {"position": "half left", "fraction": -0.5, "native": "25L", "scale": "50L..C..50R"}},
            "fl": {"pan": {"position": "half left", "fraction": -0.5, "native": "50% left",
                           "scale": "100% left..100% right"}},
        },
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Double-click the %s Pan knob value and type -32 (half left on Logic's -64..+63), Return."
                     % TRACK,
            "ableton": "Set the %s track's Pan to 25L (half left: Live's scale is 50L..C..50R, so 50L would be "
                       "HARD left)." % TRACK,
            "fl": "Set the %s channel's panning to 50%% left (half left on FL's ±100%%)." % TRACK,
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps."},
    },
    {
        "n": 17, "sub": "b", "id": "pan-half-right",
        "edit": "Pan the %s track half-way right (the plan's \"R50\")" % TRACK,
        "purpose": "Pan encoding; the same musical position in native units per DAW",
        "expected": {"pan": {"position": "half right", "fraction": 0.5}},
        "expected_by_daw": {
            "logic": {"pan": {"position": "half right", "fraction": 0.5, "native": 32, "scale": "-64..+63"}},
            "ableton": {"pan": {"position": "half right", "fraction": 0.5, "native": "25R", "scale": "50L..C..50R"}},
            "fl": {"pan": {"position": "half right", "fraction": 0.5, "native": "50% right",
                           "scale": "100% left..100% right"}},
        },
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Pan value +32 (half right on Logic's -64..+63).",
            "ableton": "Pan 25R (half right on Live's 50L..C..50R).",
            "fl": "Panning 50% right (half right on FL's ±100%).",
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps."},
    },
    {
        "n": 18, "sub": "a", "id": "mute",
        "edit": "Mute the %s track" % TRACK,
        "purpose": "Track state: mute",
        "expected": {"track_muted": True},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Click the M button on the %s track header." % TRACK,
            "ableton": "Click the %s track's Track Activator (the numbered button) off." % TRACK,
            "fl": "Mute the %s channel (its green light in the Channel rack)." % TRACK,
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps."},
    },
    {
        "n": 18, "sub": "b", "id": "solo",
        "edit": "Solo the instrument track from step 13 (leave %s muted)" % TRACK,
        "purpose": "Track state: solo",
        "expected": {"track_soloed": True},
        "target": {"holds": MIDI_13},
        "daws": {
            "logic": "Click the S button on the instrument track header.",
            "ableton": "Click the instrument track's Solo (S) button.",
            "fl": "Solo the FL Keys channel (right-click its mute light ▸ Solo); note what you used.",
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps."},
    },
    {
        "n": 19, "sub": "", "id": "insert-eq",
        "edit": "Insert the stock EQ on the %s track" % TRACK,
        "purpose": "Plugin added",
        "expected": {"plugin_added": {"kind": "eq", "stock": True}},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Channel strip ▸ first Audio FX slot ▸ EQ ▸ Channel EQ.",
            "ableton": "Drag Audio Effects ▸ EQ Eight onto the %s track." % TRACK,
            "fl": "Route the %s channel to a free Mixer insert if it is not already (note it), then load "
                  "Fruity Parametric EQ 2 into that insert's first slot." % TRACK,
        },
        "not_applicable": {"garageband": "GarageBand's per-track EQ is built in; plugin steps skipped."},
    },
    {
        "n": 20, "sub": "", "id": "eq-band-gain",
        "edit": "Raise one EQ band by +6 dB (a peak band near 1 kHz)",
        "purpose": "Plugin parameter",
        "expected": {"plugin_param_changed": {"plugin": "eq", "param": "band gain", "value_db": 6}},
        "daws": {
            "logic": "In Channel EQ, set the band nearest 1 kHz to +6.0 dB gain.",
            "ableton": "In EQ Eight, set band 4 (a bell) to +6.0 dB.",
            "fl": "In Fruity Parametric EQ 2, set band 4 to +6.0 dB.",
        },
        "not_applicable": {"garageband": "GarageBand plugin steps skipped."},
    },
    {
        "n": 21, "sub": "", "id": "volume-automation",
        "edit": "Add volume automation on the %s track: 0 dB at bar 1, -12 dB at bar 3 (two points)" % TRACK,
        "purpose": "Automation",
        "expected": {"automation_added": {"param": "volume", "points": 2}},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Press A to show automation, choose Volume on the %s track, click two points." % TRACK,
            "ableton": "Press A (automation mode), pick Mixer ▸ Track Volume on the %s track, add two breakpoints." % TRACK,
            "fl": "Right-click the %s channel's volume ▸ Create automation clip; shape it to two points." % TRACK,
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps."},
    },
    {
        "n": 22, "sub": "", "id": "marker-chorus",
        "edit": "Add a marker / locator named Chorus at bar 5",
        "purpose": "Section names",
        "expected": {"marker_added": "Chorus"},
        "daws": {
            "logic": "Move the playhead to bar 5; Navigate ▸ Other ▸ Create Marker; rename it Chorus.",
            "ableton": "Move the playhead to bar 5; Create ▸ Add Locator; rename it (⌘R) Chorus.",
            "fl": "Playlist ▸ Add time marker at bar 5 (⌥T); name it Chorus.",
        },
        "not_applicable": {"garageband": "GarageBand's arrangement track is not a marker list; skipped."},
    },
    {
        "n": 23, "sub": "a", "id": "track-colour",
        "edit": "Change the %s track's colour (first red swatch)" % TRACK,
        "purpose": "Colours for the heat strip",
        "expected": {"track_color_changed": True},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Right-click the %s track header ▸ Assign Track Color ▸ first red." % TRACK,
            "ableton": "Right-click the %s track title ▸ first red swatch." % TRACK,
            "fl": "Right-click the %s channel ▸ Color ▸ first red." % TRACK,
        },
        "not_applicable": {"garageband": "GarageBand has no user track colours."},
    },
    {
        "n": 23, "sub": "b", "id": "add-send",
        "edit": "Add a send from the %s track to a bus / return at -6 dB" % TRACK,
        "purpose": "Routing",
        "expected": {"send_added": {"level_db": -6}},
        "target": dict(TRACK_TARGET),
        "daws": {
            "logic": "Channel strip ▸ Send slot ▸ Bus ▸ Bus 1 (Logic creates Aux 1); set the send to -6 dB.",
            "ableton": "Set the %s track's Send A (Return A exists by default) to -6 dB." % TRACK,
            "fl": "Mixer: from the %s insert, route to a second free insert as a send and set the send "
                  "level to -6 dB." % TRACK,
        },
        "not_applicable": {"garageband": "GarageBand has no user sends/buses."},
    },
    {
        "n": 24, "sub": "", "id": "delete-track",
        "edit": "Delete the audio track made in step 8b — the one holding %s, NOT %s" % (PERSIAN_FILE, TRACK),
        "deletes_track": PERSIAN_FILE,
        "target": {"holds": PERSIAN_FILE},
        "purpose": "Removal",
        "expected": {"track_deleted": {"kind": "audio"}},
        "expected_by_daw": {"fl": {"channel_deleted": {"kind": "audio_clip"}}},
        "daws": {
            "logic": "Select the header of the track holding %s; Track ▸ Delete Track (⌘⌫)." % PERSIAN_FILE,
            "ableton": "Select the track holding %s; Edit ▸ Delete (⌫)." % PERSIAN_FILE,
            "fl": "Delete the %s clip from the Playlist, then the %s channel (Channel rack: right-click ▸ "
                  "Delete)." % (PERSIAN_FILE, PERSIAN_FILE[:-4]),
            "garageband": "Select the header of the track holding %s; Track ▸ Delete Track (⌘⌫)." % PERSIAN_FILE,
        },
    },
    {
        "n": 25, "sub": "a", "id": "save-copy",
        "edit": "Save a copy of the project into the same run folder as `Lab copy`",
        "purpose": "Do region and sample IDs survive a copy? Feeds the family tree",
        "expected": {"project_copied": {"name": "Lab copy"}},
        "allow_identical": True,
        "daws": {
            "logic": "File ▸ Save a Copy As… → ~/WitLab/logic/<run>/ as `Lab copy` (package; copy audio). "
                     "The original stays open.",
            "ableton": "File ▸ Save a Copy… (if offered) into the same Project folder as `Lab copy`; otherwise "
                       "Save Live Set As… `Lab copy`, then File ▸ Open the original `Lab.als` from the lab run "
                       "folder (never from Open Recent).",
            "fl": "File ▸ Save new version (FL writes a numbered copy next to Lab.flp and keeps it open); "
                  "note which file is open afterwards.",
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps."},
    },
    {
        "n": 25, "sub": "b", "id": "logic-alternative",
        "edit": "Create a project Alternative named `Alt B`",
        "purpose": "Alternatives (Logic's native branching) and ID survival",
        "expected": {"alternative_added": {"name": "Alt B"}},
        "daws": {
            "logic": "File ▸ Alternatives ▸ New Alternative… named `Alt B` (Logic switches to it).",
        },
        "not_applicable": {
            "ableton": "Alternatives are Logic-only.",
            "fl": "Alternatives are Logic-only.",
            "garageband": "Alternatives are Logic-only.",
        },
    },
    {
        "n": 26, "sub": "", "id": "bounce-30s",
        "edit": "Bounce / export the first 30 s into <run>/lab-bounces/, then save the project",
        "purpose": "The Ears tier; pairing a bounce with a save",
        "expected": {"bounce_exported": {"seconds": 30, "folder": "lab-bounces"}},
        "allow_identical": True,
        "daws": {
            "logic": "Set a cycle over the first 30 s; File ▸ Bounce ▸ Project or Section (⌘B), PCM WAV 16-bit, "
                     "destination ~/WitLab/logic/<run>/lab-bounces/. Then save.",
            "ableton": "Loop brace over the first 30 s; File ▸ Export Audio/Video (⇧⌘R), WAV 16-bit, into "
                       "~/WitLab/ableton/<run>/lab-bounces/. Then save.",
            "fl": "Select the first 30 s in the Playlist; File ▸ Export ▸ WAVE file… into "
                  "~/WitLab/fl/<run>/lab-bounces/. Then save.",
        },
        "not_applicable": {"garageband": "Kept to ~10 GarageBand steps."},
    },
]


# --------------------------------------------------------------------------- #
# accessors
# --------------------------------------------------------------------------- #


def step_key(step: Dict) -> str:
    """"03", "08a", "17b" — sortable, used in capture directory names."""
    return "%02d%s" % (step["n"], step.get("sub", ""))


def step_label(step: Dict) -> str:
    """Directory name in the corpus: "03-tempo-124", "17a-pan-left-50"."""
    return "%s-%s" % (step_key(step), step["id"])


def applicable(step: Dict, daw: str) -> bool:
    return daw in step["daws"]


def for_daw(daw: str) -> List[Dict]:
    """The steps that apply to `daw`, flattened for a manifest (instructions for that DAW only)."""
    out = []
    for s in STEPS:
        if not applicable(s, daw):
            continue
        out.append({
            "key": step_key(s),
            "n": s["n"],
            "sub": s.get("sub", ""),
            "id": s["id"],
            "label": step_label(s),
            "edit": s["edit"],
            "purpose": s["purpose"],
            "expected": s.get("expected_by_daw", {}).get(daw, s["expected"]),
            "target": s.get("target"),
            "allow_identical": bool(s.get("allow_identical", False)),
            "instructions": s["daws"][daw],
            "save_hint": SAVE_HINT[daw],
        })
    return out


def find(daw: str, key: str) -> Optional[Dict]:
    """Look up an applicable step by key ("8a", "08a", "3", "03") or id ("tempo-124")."""
    want = key.strip().lower()
    for s in for_daw(daw):
        k = s["key"]
        if want in (k, k.lstrip("0"), s["id"]):
            return s
    return None


def format_instructions(step: Dict, daw: str, total: int, index: int) -> str:
    lines = [
        "Step %s (%d of %d for %s) — %s" % (step["key"], index, total, daw, step["id"]),
        "  Edit:     %s" % step["edit"],
        "  Purpose:  %s" % step["purpose"],
        "  Do:       %s" % step["instructions"],
        "  %s" % step["save_hint"],
        "  Expected: %s" % json.dumps(step["expected"], ensure_ascii=False),
    ]
    if step.get("target"):
        lines.append("  Target:   %s" % json.dumps(step["target"], ensure_ascii=False))
    flag = " (identical bytes allowed for this step)" if step["allow_identical"] else ""
    lines.append("  Then:     python3 tools/lab/capture.py watch%s" % flag)
    return "\n".join(lines)


# --------------------------------------------------------------------------- #
# validation
# --------------------------------------------------------------------------- #


def _expected_keys(expected: Dict) -> List[str]:
    return list(expected.keys())


def track_names(steps: List[Dict]) -> Dict[str, set]:
    """
    For every track the script creates (keyed by what it holds): every name it can
    carry at some point. A track holding an imported file may be named after that
    file (FL does this), so the file's stem is always a possible name.
    """
    names: Dict[str, set] = {}
    for s in steps:
        held = s.get("adds_track")
        if held:
            names.setdefault(held, set())
            if held.lower().endswith(".wav"):
                names[held].add(unicodedata.normalize("NFC", held[:-4]))
        rename = s.get("renames_track")
        if rename and rename["holds"] in names:
            names[rename["holds"]].add(unicodedata.normalize("NFC", rename["to"]))
    return names


def _track_problems(steps: List[Dict]) -> List[str]:
    problems = []
    names = track_names(steps)
    for s in steps:
        k = step_key(s)
        rename = s.get("renames_track")
        if rename and rename["holds"] not in names:
            problems.append("%s: renames a track that no step adds (%s)" % (k, rename["holds"]))
        if s.get("deletes_track") and s["deletes_track"] not in names:
            problems.append("%s: deletes a track that no step adds (%s)" % (k, s["deletes_track"]))
        target = s.get("target") or {}
        if "track" in target:
            held = target.get("holds")
            if held is None:
                problems.append("%s: a track target must also say what the track holds" % k)
            elif held not in names:
                problems.append("%s: targets a track holding %s, which no step adds" % (k, held))
            elif unicodedata.normalize("NFC", target["track"]) not in names[held]:
                problems.append("%s: the track holding %s is never called %s" % (k, held, target["track"]))
        elif "holds" in target and target["holds"] not in names:
            problems.append("%s: targets a track holding %s, which no step adds" % (k, target["holds"]))
    held = sorted(names)
    for i, a in enumerate(held):
        for b in held[i + 1:]:
            clash = names[a] & names[b]
            if clash:
                problems.append("two different tracks (holding %s and %s) could both be called %s — every "
                                "'the %s track' instruction would be ambiguous"
                                % (a, b, ", ".join(sorted(clash)), sorted(clash)[0]))
    return problems


def validate(steps: Optional[List[Dict]] = None) -> List[str]:
    """Every structural rule the data must satisfy. Empty list = valid."""
    steps = STEPS if steps is None else steps
    problems = []
    seen_keys, seen_ids = set(), set()
    numbers = set()
    daws = set(SAVE_HINT)
    for s in steps:
        k = step_key(s)
        if k in seen_keys:
            problems.append("duplicate step key %s" % k)
        seen_keys.add(k)
        if s["id"] in seen_ids:
            problems.append("duplicate step id %s" % s["id"])
        seen_ids.add(s["id"])
        numbers.add(s["n"])
        for field in ("edit", "purpose", "expected", "daws"):
            if not s.get(field):
                problems.append("%s: missing %s" % (k, field))
        unknown_daws = (set(s["daws"]) | set(s.get("not_applicable", {}))) - daws
        if unknown_daws:
            problems.append("%s: unknown DAW(s) %s" % (k, sorted(unknown_daws)))
        overlap = set(s["daws"]) & set(s.get("not_applicable", {}))
        if overlap:
            problems.append("%s: both applicable and not applicable for %s" % (k, sorted(overlap)))
        missing = daws - set(s["daws"]) - set(s.get("not_applicable", {}))
        if missing:
            problems.append("%s: no instruction and no not-applicable reason for %s" % (k, sorted(missing)))
        for daw, text in s["daws"].items():
            if not text or not text.strip():
                problems.append("%s: empty instruction for %s" % (k, daw))
        expectations = [s["expected"], *s.get("expected_by_daw", {}).values()]
        for exp in expectations:
            bad = set(_expected_keys(exp)) - EXPECTED_KEYS
            if bad:
                problems.append("%s: expected keys outside the vocabulary: %s" % (k, sorted(bad)))
        for daw in s.get("expected_by_daw", {}):
            if daw not in s["daws"]:
                problems.append("%s: expected_by_daw for non-applicable %s" % (k, daw))
    if numbers != set(range(1, 27)):
        problems.append("plan rows 1-26 not all present: missing %s" % sorted(set(range(1, 27)) - numbers))
    order = [(s["n"], s.get("sub", "")) for s in steps]
    if order != sorted(order):
        problems.append("steps are not in plan order")
    problems.extend(_track_problems(steps))
    return problems


# --------------------------------------------------------------------------- #
# cli
# --------------------------------------------------------------------------- #


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Print or check the Rosetta lab edit script.")
    ap.add_argument("--daw", choices=sorted(SAVE_HINT), help="only the steps for this DAW")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--check", action="store_true", help="validate the data and exit")
    args = ap.parse_args(argv)

    if args.check:
        problems = validate()
        for p in problems:
            print("problem: %s" % p)
        if problems:
            return 1
        counts = ", ".join("%s %d" % (d, len(for_daw(d))) for d in ("logic", "ableton", "fl", "garageband"))
        print("ok: %d steps over plan rows 1-26 (%s)" % (len(STEPS), counts))
        return 0

    if args.json:
        data = for_daw(args.daw) if args.daw else STEPS
        print(json.dumps(data, indent=2, ensure_ascii=False))
        return 0

    daws = [args.daw] if args.daw else ["logic", "ableton", "fl", "garageband"]
    for daw in daws:
        steps = for_daw(daw)
        print("=== %s: %d steps ===" % (daw, len(steps)))
        for i, s in enumerate(steps, 1):
            print(format_instructions(s, daw, len(steps), i))
            print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
