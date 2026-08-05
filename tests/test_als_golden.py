"""
Golden / characterisation tests for the diff's user-facing text.

The diff output *is* the product. docs/EXPERIMENTS.md quotes it verbatim, the
README shows it, and a musician reads it. A refactor that changes "MIX~" to
"MIXER~", drops the two-space gutter, or reorders the blocks would break every
one of those without any other test noticing.

These tests are characterisation tests: they encode what the tool does today, not
what it ideally should do. Changing them is allowed — changing them *by accident*
is what they prevent. If one fails, decide whether the new text is intended and
whether docs/EXPERIMENTS.md needs updating in the same commit.
"""

from __future__ import annotations

import subprocess
import sys
import textwrap

from factories.als import Clip, LiveSet, Track, simple_set


def kitchen_sink():
    """One 'before' set and one 'after' set that exercise every line type."""
    before = LiveSet(
        tempo=120.0,
        tracks=[
            Track(
                id="100",
                name="Drums",
                color="13",
                volume=1.0,
                pan=0.0,
                devices=["Eq8"],
                clips=[
                    Clip(id="10", name="kick", start=0.0, end=16.0,
                         sample="Samples/Imported/old kick.wav"),
                    Clip(id="11", name="hat", start=16.0, end=32.0,
                         sample="Samples/Imported/hat.wav"),
                ],
            ),
            Track(
                id="101",
                name="Bass",
                color="26",
                volume=0.794,
                clips=[Clip(id="20", name="sub", start=0.0, end=32.0,
                            sample="Samples/Recorded/sub.wav")],
            ),
            Track(id="102", name="Strings", kind="MidiTrack", automation_lanes=1,
                  clips=[Clip(id="30", name="pad", start=8.0, end=24.0, notes=12)]),
            Track(id="103", name="Scratch", clips=[]),
        ],
    )

    after = before.copy()
    after.tempo = 124.0
    after.remove_track("Scratch")
    after.tracks.append(Track(id="104", name="Vox", kind="AudioTrack"))

    drums = after.track("Drums")
    drums.clip("10").sample = "Samples/Imported/Kick 01.wav"  # a genuine rename
    drums.clip("11").start, drums.clip("11").end = 16.0, 30.0
    drums.devices = ["Eq8", "Compressor2"]
    drums.pan = -0.15

    bass = after.track("Bass")
    bass.name = "Sub Bass"
    bass.volume = 0.525
    bass.clips.append(Clip(id="21", name="", start=32.0, end=40.0,
                           sample="Samples/Imported/drop.wav"))

    strings = after.track("Strings")
    strings.speaker = False
    strings.color = "7"
    strings.automation_lanes = 3
    strings.clip("30").notes = 20
    strings.clip("30").disabled = True

    return before, after


GOLDEN_DIFF = [
    "TEMPO   120.0 -> 124.0 BPM",
    "SAMPLE~ 'old kick.wav' -> 'Kick 01.wav'  (1 clip reference(s))",
    "TRACK+  added 'Vox' (AudioTrack)",
    "TRACK-  removed 'Scratch'",
    "MIX~    [Drums] pan: 0.0 -> -0.15",
    "FX+     [Drums] added: Compressor2",
    "CLIP~   [Drums] 'hat' 16.0-32.0 -> 16.0-30.0",
    "TRACK~  renamed 'Bass' -> 'Sub Bass'",
    "MIX~    [Sub Bass] volume: 0.794 -> 0.525",
    "CLIP+   [Sub Bass] added 'drop.wav' at bar 32.0",
    "MIX~    [Strings] output enabled: true -> false",
    "MIX~    [Strings] color: 13 -> 7",
    "AUTO~   [Strings] automation lanes 1 -> 3",
    "MIDI~   [Strings] note count 12 -> 20",
    "CLIP~   [Strings] 'pad' muted",
]


def test_golden_diff_lines(diff_of):
    before, after = kitchen_sink()
    assert diff_of(before, after) == GOLDEN_DIFF


def test_every_line_type_the_differ_can_emit_is_covered_by_the_golden(diff_of):
    """
    If a new line type is added, the golden above must grow. This asserts the
    complete vocabulary, so a new prefix cannot slip in untested.
    """
    prefixes = sorted({line.split()[0] for line in GOLDEN_DIFF})
    assert prefixes == [
        "AUTO~",
        "CLIP+",
        "CLIP~",
        "FX+",
        "MIDI~",
        "MIX~",
        "SAMPLE~",
        "TEMPO",
        "TRACK+",
        "TRACK-",
        "TRACK~",
    ]


def test_the_remaining_line_types_have_their_own_golden(diff_of):
    """CLIP-, FX-, FX~ and 'unmuted' do not co-occur with the set above."""
    a = simple_set()
    b = a.copy()
    b.track("Drums").devices = []
    b.track("Drums").clips = [c for c in b.track("Drums").clips if c.id != "11"]
    b.track("Bass").clips[0].disabled = False
    assert diff_of(a, b) == [
        "FX-     [Drums] removed: Eq8",
        "CLIP-   [Drums] removed 'hat loop' at bar 16.0",
    ]

    c = simple_set()
    c.track("Drums").devices = ["Eq8", "AutoFilter"]
    d = c.copy()
    d.track("Drums").devices = ["AutoFilter", "Eq8"]
    assert diff_of(c, d) == ["FX~     [Drums] device chain reordered"]

    e = simple_set()
    e.track("Drums").clip("10").disabled = True
    f = e.copy()
    f.track("Drums").clip("10").disabled = False
    assert diff_of(e, f) == ["CLIP~   [Drums] 'kick loop' unmuted"]


def test_prefix_column_is_eight_characters_wide():
    """The gutter is what makes the output scannable; keep it aligned."""
    for line in GOLDEN_DIFF:
        assert line[:8] == line[:8].ljust(8), line
        assert line[7] == " " or line.startswith("SAMPLE~ "), line
        assert not line[8:9].isspace(), "double gutter in %r" % line


# --------------------------------------------------------------------------- #
# report() — the printed form
# --------------------------------------------------------------------------- #


def test_no_change_message_is_exact(als_diff, capsys, tmp_path):
    """
    Quoted verbatim in docs/EXPERIMENTS.md 4 and README. If this string changes,
    those documents are wrong.
    """
    a = simple_set()
    pa, pb = str(tmp_path / "a.als"), str(tmp_path / "b.als")
    a.write(pa)
    a.copy().write(pb)

    capsys.readouterr()
    count = als_diff.report(pa, pb)
    out = capsys.readouterr().out

    assert count == 0
    assert out == "  no musical change detected (view / bookkeeping only)\n"


def test_report_layout_and_return_value(als_diff, capsys, tmp_path):
    before, after = kitchen_sink()
    pa, pb = str(tmp_path / "a.als"), str(tmp_path / "b.als")
    before.write(pa)
    after.write(pb)

    capsys.readouterr()
    count = als_diff.report(pa, pb)
    out = capsys.readouterr().out

    assert count == len(GOLDEN_DIFF)
    expected = ["  %d semantic change(s)" % len(GOLDEN_DIFF)]
    expected += ["    " + line for line in GOLDEN_DIFF]
    assert out.splitlines() == expected


def test_report_truncates_at_the_limit(als_diff, capsys, tmp_path):
    before, after = kitchen_sink()
    pa, pb = str(tmp_path / "a.als"), str(tmp_path / "b.als")
    before.write(pa)
    after.write(pb)

    capsys.readouterr()
    als_diff.report(pa, pb, limit=4)
    lines = capsys.readouterr().out.splitlines()

    assert lines[0] == "  %d semantic change(s)" % len(GOLDEN_DIFF)
    assert lines[1:5] == ["    " + line for line in GOLDEN_DIFF[:4]]
    assert lines[5] == "    ... and %d more" % (len(GOLDEN_DIFF) - 4)
    assert len(lines) == 6


# --------------------------------------------------------------------------- #
# the command line, end to end
# --------------------------------------------------------------------------- #


def run_cli(experiments_dir, *args):
    return subprocess.run(
        [sys.executable, str(experiments_dir / "als_semantic_diff.py"), *args],
        capture_output=True,
        text=True,
    )


def test_cli_pairwise_output(experiments_dir, tmp_path):
    before, after = kitchen_sink()
    pa, pb = str(tmp_path / "a.als"), str(tmp_path / "b.als")
    before.write(pa)
    after.write(pb)

    proc = run_cli(experiments_dir, pa, pb)
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.splitlines()[0] == "  %d semantic change(s)" % len(GOLDEN_DIFF)
    assert proc.stdout.splitlines()[1:] == ["    " + line for line in GOLDEN_DIFF]


def test_cli_chain_mode_output(experiments_dir, tmp_path):
    """
    --chain is how CONTRIBUTING tells a musician to try Wit on their own Backup
    folder, so its header format is user-visible too.
    """
    v1 = simple_set()
    v2 = v1.copy()
    v2.track("Bass").volume = 0.5
    v3 = v2.copy()  # a musically empty save
    v3.scroll_x = 900
    for name, ls in (("001.als", v1), ("002.als", v2), ("003.als", v3)):
        ls.write(tmp_path / name)

    proc = run_cli(experiments_dir, "--chain", str(tmp_path / "*.als"))
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout == textwrap.dedent(
        """\
        ### 001.als  ->  002.als
          1 semantic change(s)
            MIX~    [Bass] volume: 0.794 -> 0.5

        ### 002.als  ->  003.als
          no musical change detected (view / bookkeeping only)

        """
    )


def test_cli_requires_two_files(experiments_dir, tmp_path):
    proc = run_cli(experiments_dir, str(tmp_path / "only.als"))
    assert proc.returncode != 0
    assert "provide OLD and NEW" in proc.stderr


def test_cli_chain_needs_at_least_two_matches(experiments_dir, tmp_path):
    simple_set().write(tmp_path / "only.als")
    proc = run_cli(experiments_dir, "--chain", str(tmp_path / "*.als"))
    assert proc.returncode != 0
    assert "need at least 2 files" in proc.stderr


def test_cli_help_names_the_tool(experiments_dir):
    """CI runs `--help` on every prototype; keep it meaningful, not just exit 0."""
    proc = run_cli(experiments_dir, "--help")
    assert proc.returncode == 0
    assert "Semantic diff for Ableton Live sets" in proc.stdout
    assert "--chain" in proc.stdout
